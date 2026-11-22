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

fn catalog_reminders(session: &AgentSession) -> Vec<Value> {
    reminders(session)
        .into_iter()
        .filter(|entry| entry.pointer("/details/key").and_then(Value::as_str) == Some("oino/tools"))
        .collect()
}

fn tool(name: &str) -> oino_core::extensions::api::ToolSpec {
    serde_json::from_value(json!({"name":name,"label":name,"description":name,
        "parameters":{"type":"object","properties":{},"additionalProperties":false}}))
    .unwrap()
}

async fn register_tool(session: &AgentSession, owner: &str, name: &str) {
    session
        .host_call(
            owner,
            session.runner().call_context(),
            HostOp::RegisterTool(tool(name)),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn tool_catalog_changes_coalesce_into_one_diff_reminder() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 4]);
    let session = TestSession {
        extensions: vec![Arc::new(FakeExtension::new(
            "mcp",
            oino_core::extensions::api::Registration::default(),
        ))],
        ..TestSession::default()
    }
    .build(provider.clone())
    .await;
    turn(&session).await;
    assert_eq!(catalog_reminders(&session), [] as [Value; 0]);

    for n in 0..5 {
        register_tool(&session, "mcp", &format!("mcp_{n}")).await;
    }
    turn(&session).await;
    let reminders = catalog_reminders(&session);
    assert_eq!(reminders.len(), 1, "one reminder per registration batch");
    let text = reminders[0]["content"].to_string();
    assert!(text.contains("Tools added:"));
    for n in 0..5 {
        assert!(text.contains(&format!("mcp_{n}")), "{text}");
    }
    assert!(!text.contains("Unified tool catalog:"));
    // The diff is delivered with the request that follows the registration batch.
    let request = provider.requests()[1].to_string();
    assert!(request.contains("Tools added:"), "{request}");
    assert!(request.contains("mcp_4"), "{request}");

    // The catalog is unchanged: a later turn appends nothing.
    turn(&session).await;
    assert_eq!(catalog_reminders(&session).len(), 1);

    // Removing a tool announces only the removal.
    session
        .host_call(
            "mcp",
            session.runner().call_context(),
            HostOp::UnregisterTool("mcp_0".into()),
        )
        .await
        .unwrap();
    turn(&session).await;
    let reminders = catalog_reminders(&session);
    assert_eq!(reminders.len(), 2);
    let text = reminders[1]["content"].to_string();
    assert!(text.contains("Tools removed:"));
    assert!(text.contains("mcp_0"));
}

#[tokio::test]
async fn append_order_and_reload() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 3]);
    let directory = tempfile::tempdir().unwrap();
    let session = TestSession {
        session_dir: Some(directory.path().to_owned()),
        extensions: vec![Arc::new(FakeExtension::new(
            "test",
            oino_core::extensions::api::Registration::default(),
        ))],
        ..TestSession::default()
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
                oino_core::extensions::api::Registration::default(),
            )),
            Arc::new(FakeExtension::new(
                "unipi.mcp",
                oino_core::extensions::api::Registration::default(),
            )),
        ],
        ..TestSession::default()
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
