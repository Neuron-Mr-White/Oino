//! Correlated host approvals reuse the existing keyboard-accessible dialog renderer.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod common;
use common::{Harness, TestHost};
use oino_tui::Effect;
use serde_json::json;

#[tokio::test]
async fn approval_once_always_deny_and_cancel() {
    for (keys, expected) in [
        (vec!["enter"], "allow_once"),
        (vec!["down", "enter"], "always"),
        (vec!["down", "down", "enter"], "deny"),
        (vec!["escape"], "deny"),
    ] {
        let mut h = Harness::new(TestHost::default()).await;
        h.effects.clear();
        let request = json!({"type":"approval_request", "requestId":"r1", "sessionId":"s-1", "tool":"bash", "arguments":{"command":"git push", "summary":"Push branch"}, "riskLevel":"high", "ceiling":"medium", "reason":"needs Autonomy High"});
        h.json_event(request.clone()).await;
        h.json_event(request).await;
        let screen = h.frame(70, 24);
        assert!(screen.contains("Approval needed"), "{screen}");
        for key in keys {
            h.key(key).await;
        }
        let responses: Vec<_> = h
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::Send {
                    command,
                    concurrent,
                    ..
                } if command.kind() == "approval_response" => {
                    assert!(*concurrent, "approval must bypass a pending command queue");
                    Some(serde_json::to_value(command).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["requestId"], "r1");
        assert_eq!(responses[0]["decision"], expected);
        assert!(!h.frame(40, 15).contains("Approval needed"));
    }
}

#[tokio::test]
async fn foreground_child_approval_uses_parent_channel() {
    let mut h = Harness::new(TestHost::default()).await;
    h.json_event(json!({"type":"approval_request", "requestId":"child-r1", "sessionId":"child-session", "tool":"bash", "arguments":{"command":"git push"}, "riskLevel":"high", "ceiling":"medium", "reason":"needs Autonomy High"})).await;
    assert!(h.frame(70, 24).contains("Approval needed"));
    h.key("escape").await;
    assert!(h.effects.iter().any(|effect| matches!(
        effect,
        Effect::Send {
            command: oino_client::RpcCommand::Agent(
                oino_client::AgentCommand::ApprovalResponse { .. }
            ),
            ..
        }
    )));
}
