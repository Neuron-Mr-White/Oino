//! Encoded context reuse must preserve sequential mutation and system visibility.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests fail on unexpected extension errors"
)]
use indexmap::IndexMap;
use oino_ai::{AgentMessage, SystemMessage, UserContentField, UserMessage, UserTextField};
use oino_core::{
    events::{ExtensionErrorEvent, OinoEvent},
    extensions::{
        api::{CallContext, Registration},
        runner::{ErrorSink, ExtensionRunner, ExtensionSet, LoadedExtension},
    },
    testing::FakeExtension,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
struct Errors;
impl ErrorSink for Errors {
    fn report(&self, error: ExtensionErrorEvent) {
        panic!("{error:?}");
    }
    fn disabled(&self, event: OinoEvent) {
        panic!("{event:?}");
    }
}
#[tokio::test]
async fn six_subscribers_observe_prior_edits_and_system_rules() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut extensions = Vec::new();
    for n in 0..6 {
        let seen = seen.clone();
        let registration = Registration {
            events: vec!["context".into(), "context_with_system".into()],
            ..Registration::default()
        };
        let extension = FakeExtension::new(&format!("subscriber-{n}"), registration.clone())
            .on_event(move |kind, payload| {
                seen.lock()
                    .unwrap()
                    .push((n, kind.to_owned(), payload.clone()));
                if n == 2 && kind == "context" {
                    let mut messages = payload["messages"].clone();
                    messages[0]["content"] = json!("changed");
                    Ok(Some(json!({"messages": messages})))
                } else if n == 4 {
                    Ok(Some(json!({"messages":payload["messages"]})))
                } else {
                    Ok(None)
                }
            });
        extensions.push(Arc::new(LoadedExtension::new(
            Arc::new(extension),
            registration,
        )));
    }
    let runner = ExtensionRunner::new(
        Arc::new(ExtensionSet::new(extensions)),
        CallContext::default(),
        Arc::new(Errors),
        IndexMap::new(),
    );
    let system = AgentMessage::System(SystemMessage {
        content: UserTextField::Text("system".into()),
        ..SystemMessage::default()
    });
    let messages = vec![
        system.clone(),
        AgentMessage::User(UserMessage {
            author: None,
            content: UserContentField::Text("original".into()),
            timestamp: 1,
        }),
    ];
    let result = runner.emit_context(messages).await;
    assert_eq!(result[0], system);
    assert_eq!(
        serde_json::to_value(&result[1]).unwrap()["content"],
        "changed"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 12);
    for (index, (n, kind, payload)) in seen.iter().enumerate() {
        assert_eq!(*n, index % 6);
        assert_eq!(
            kind,
            if index < 6 {
                "context"
            } else {
                "context_with_system"
            }
        );
        assert_eq!(
            payload["messages"].as_array().unwrap().len(),
            if index < 6 { 1 } else { 2 }
        );
        if index < 6 {
            assert_eq!(
                payload["messages"][0]["content"],
                if index <= 2 { "original" } else { "changed" }
            );
        }
    }
}
