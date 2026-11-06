//! Append-only extension reminder acceptance through the native host boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use oino_core::extensions::host::HostOp;
use oino_core::session::{AgentSession, PromptOptions};
use oino_core::testing::{FakeExtension, FauxProvider, FauxResponse, TestSession};
use serde_json::{Value, json};

async fn turn(session: &AgentSession) {
    session
        .prompt("hello".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

async fn append(session: &AgentSession, owner: &str, key: &str, text: &str) -> Value {
    session
        .host_call(
            owner,
            session.runner().call_context(),
            HostOp::AppendReminder {
                key: key.into(),
                text: text.into(),
            },
        )
        .await
        .unwrap()
}

fn reminders(session: &AgentSession) -> Vec<Value> {
    session
        .entries()
        .iter()
        .map(|entry| serde_json::to_value(entry).unwrap())
        .filter(|entry| entry["customType"] == "oino.system-reminder")
        .collect()
}

#[tokio::test]
async fn append_order_and_reload() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 3]);
    let directory = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(directory.path().to_owned()),
        extensions: vec![Arc::new(FakeExtension::new("test", Default::default()))],
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session).await;
    let before = session.entries();
    let first = append(&session, "test", "test/state", "first").await;
    let second = append(&session, "test", "test/state", "second").await;
    assert_eq!(first["appended"], true);
    assert_ne!(first["id"], second["id"]);
    assert!(
        session
            .host_call(
                "test",
                session.runner().call_context(),
                HostOp::AppendReminder {
                    key: "other/state".into(),
                    text: "spoof".into(),
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(&session.entries()[..before.len()]).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    let persisted = reminders(&session);
    assert_eq!(persisted.len(), 2);
    assert!(persisted[0]["content"].as_str().unwrap().contains("first"));
    assert!(persisted[1]["content"].as_str().unwrap().contains("second"));
    session.reload().await.unwrap();
    assert_eq!(reminders(&session), persisted);
    turn(&session).await;
    let requests = provider.requests();
    assert_eq!(
        requests[0]["context"]["messages"][0],
        requests[1]["context"]["messages"][0]
    );
    assert_eq!(reminders(&session), persisted);
}

#[tokio::test]
async fn skill_mcp_catalog_updates() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 2]);
    let session = TestSession {
        extensions: vec![
            Arc::new(FakeExtension::new(
                "unipi.skill-registry",
                Default::default(),
            )),
            Arc::new(FakeExtension::new("unipi.mcp", Default::default())),
        ],
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    turn(&session).await;
    let before = session.entries();
    append(
        &session,
        "unipi.skill-registry",
        "unipi.skill-registry/skills",
        "Selected skill: example. Read /skills/example/SKILL.md.",
    )
    .await;
    let tool = serde_json::from_value(json!({"name":"mcp_example", "label":"example", "description":"MCP example", "parameters":{"type":"object"}})).unwrap();
    session
        .host_call(
            "unipi.mcp",
            session.runner().call_context(),
            HostOp::RegisterTool(tool),
        )
        .await
        .unwrap();
    append(
        &session,
        "unipi.mcp",
        "unipi.mcp/catalog",
        "MCP catalog update: mcp_example is available as a deferred tool.",
    )
    .await;
    assert_eq!(
        serde_json::to_value(&session.entries()[..before.len()]).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    turn(&session).await;
    let requests = provider.requests();
    assert_eq!(
        requests[0]["context"]["messages"][0],
        requests[1]["context"]["messages"][0]
    );
    let prompt = requests[1]["context"]["messages"][0].to_string();
    assert!(!prompt.contains("/skills/example"));
    assert!(
        !session
            .active_tool_names()
            .contains(&"mcp_example".to_owned())
    );
    let tail = serde_json::to_string(&reminders(&session)).unwrap();
    assert!(tail.contains("/skills/example/SKILL.md"));
    assert!(tail.contains("MCP catalog update"));
}
