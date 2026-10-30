//! Per-session crash protection (core spec §2.3, WP-C5): a trapping or panicking
//! extension is disabled for that session only, and a panicking drive task publishes
//! `SessionCrashed` while the session stays usable and sibling sessions are unaffected.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use oino_agent::AgentToolResult;
use oino_core::events::{ClientEvent, ExtensionDisabledReason, OinoEvent, RunState};
use oino_core::extensions::api::{ExtensionInstance, Registration, ToolSpec};
use oino_core::hub::EventStream;
use oino_core::session::{AgentSession, PromptDisposition, PromptOptions};
use oino_core::testing::{FakeExtension, FauxProvider, FauxResponse, TestSession};
use serde_json::json;

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        exposure: oino_core::extensions::api::ToolExposure::Direct,
        default_active: None,
        execution_mode: None,
        namespace: None,
        annotations: None,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    }
}

/// Collecting event watcher: nothing published after subscription is ever lost.
struct Events {
    stream: EventStream,
    seen: Vec<ClientEvent>,
}

impl Events {
    fn new(session: &AgentSession) -> Self {
        Self {
            stream: session.subscribe(),
            seen: Vec::new(),
        }
    }

    fn pump(&mut self) {
        self.seen.extend(self.stream.drain_now());
    }

    /// Waits (bounded) for an event matching `of`.
    async fn wait(&mut self, of: fn(&ClientEvent) -> bool) -> ClientEvent {
        let deadline = tokio::time::sleep(Duration::from_secs(10));
        tokio::pin!(deadline);
        loop {
            self.pump();
            if let Some(event) = self.seen.iter().find(|event| of(event)) {
                return event.clone();
            }
            tokio::select! {
                () = &mut deadline => panic!("timed out waiting for event"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    }

    fn has(&mut self, of: fn(&ClientEvent) -> bool) -> bool {
        self.pump();
        self.seen.iter().any(of)
    }
}

fn is_session_crashed(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::Oino(OinoEvent::SessionCrashed { .. }))
}

fn is_extension_disabled(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::Oino(OinoEvent::ExtensionDisabled { .. })
    )
}

fn is_extension_error(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::ExtensionError(_))
}

fn is_run_state_idle(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::Oino(OinoEvent::RunState {
            state: RunState::Idle,
            ..
        })
    )
}

async fn run_idle(session: &AgentSession, text: &str) {
    session
        .prompt(text.into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicking_handler_disables_extension_for_that_session_only() {
    // One shared instance, two sessions: the input handler panics only on "trap …".
    let extension = Arc::new(
        FakeExtension::new(
            "ext",
            Registration {
                tools: vec![tool("noop")],
                events: vec!["input".into()],
                ..Registration::default()
            },
        )
        .on_event(|_kind, payload| {
            if payload["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("trap"))
            {
                panic!("handler exploded");
            }
            Ok(None)
        }),
    );
    let provider_a = FauxProvider::new(vec![
        FauxResponse::Text("a1".into()),
        FauxResponse::Text("a2".into()),
    ]);
    let provider_b = FauxProvider::new(vec![FauxResponse::Text("b1".into())]);
    let session_a = TestSession {
        extensions: vec![Arc::clone(&extension) as Arc<dyn ExtensionInstance>],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider_a))
    .await;
    let session_b = TestSession {
        extensions: vec![Arc::clone(&extension) as Arc<dyn ExtensionInstance>],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider_b))
    .await;
    assert!(
        session_a
            .runner()
            .tools()
            .iter()
            .any(|t| t.spec.name == "noop")
    );
    assert!(session_a.runner().has_handlers("input"));

    // Session A: the panicking handler disables the extension for A and emits both events.
    let mut events_a = Events::new(&session_a);
    session_a
        .prompt(
            "trap me".into(),
            PromptOptions {
                expand_prompt_templates: false,
                ..PromptOptions::default()
            },
        )
        .await
        .unwrap();
    let disabled = events_a.wait(is_extension_disabled).await;
    assert!(events_a.has(is_extension_error), "extension_error follows");
    let ClientEvent::Oino(OinoEvent::ExtensionDisabled {
        session_id,
        extension_id,
        reason,
    }) = &disabled
    else {
        panic!("not an ExtensionDisabled event");
    };
    assert_eq!(session_id, &session_a.session_id());
    assert_eq!(extension_id, "ext");
    assert_eq!(*reason, ExtensionDisabledReason::Panic);

    tokio::time::timeout(Duration::from_secs(10), session_a.wait_for_idle())
        .await
        .unwrap();
    assert!(session_a.runner().is_disabled("ext"));
    assert!(!session_b.runner().is_disabled("ext"));
    assert!(!session_a.runner().has_handlers("input"));
    assert!(
        !session_a
            .runner()
            .tools()
            .iter()
            .any(|t| t.spec.name == "noop"),
        "the disabled extension's tools disappear for A"
    );
    assert_eq!(
        session_a.runner().disabled_extensions(),
        vec!["ext".to_owned()]
    );

    // A's second prompt no longer dispatches to the extension (no new received events).
    let received_after_disable = extension.received().len();
    run_idle(&session_a, "again").await;
    assert_eq!(extension.received().len(), received_after_disable);

    // Session B (same instance, same process) still dispatches.
    run_idle(&session_b, "hello b").await;
    assert!(!session_b.runner().is_disabled("ext"));
    assert!(session_b.runner().has_handlers("input"));
    assert!(
        session_b
            .runner()
            .tools()
            .iter()
            .any(|t| t.spec.name == "noop")
    );
    assert!(
        extension
            .received_contexts()
            .iter()
            .any(|(kind, cx)| kind == "input" && cx.session_id == session_b.session_id()),
        "session B still dispatches to the shared instance"
    );
    assert_eq!(
        session_b.last_assistant_text().as_deref(),
        Some("b1"),
        "session B completed its run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicking_tool_body_crashes_drive_run_session_recovers_and_siblings_survive() {
    // The tool body panics inside the agent loop: the drive task panics (X-PANIC), not the
    // process. `SessionCrashed` is published, the run book resets, the next prompt works,
    // and a sibling session sharing the instance is unaffected.
    let extension = Arc::new(
        FakeExtension::new(
            "ext",
            Registration {
                tools: vec![tool("boom")],
                ..Registration::default()
            },
        )
        .on_tool(|name, _| {
            if name != "boom" {
                return Ok(AgentToolResult::text("ok"));
            }
            panic!("tool body exploded");
        }),
    );
    let provider_a = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "boom".into(),
            serde_json::from_value(json!({})).unwrap(),
        )]),
        FauxResponse::Text("recovered".into()),
    ]);
    let provider_b = FauxProvider::new(vec![FauxResponse::Text("sibling ok".into())]);
    let session_a = TestSession {
        extensions: vec![Arc::clone(&extension) as Arc<dyn ExtensionInstance>],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider_a))
    .await;
    let session_b = TestSession {
        extensions: vec![Arc::clone(&extension) as Arc<dyn ExtensionInstance>],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider_b))
    .await;
    let mut events_a = Events::new(&session_a);

    assert_eq!(
        session_a
            .prompt("go".into(), PromptOptions::default())
            .await
            .unwrap(),
        PromptDisposition::Started
    );
    let crashed = events_a.wait(is_session_crashed).await;
    assert!(events_a.has(is_run_state_idle), "run_state idle follows");
    let ClientEvent::Oino(OinoEvent::SessionCrashed { error }) = &crashed else {
        panic!("not a SessionCrashed event");
    };
    assert!(error.contains("tool body exploded"), "{error}");

    // The drive task reset the run book: idle again and a new prompt is accepted.
    tokio::time::timeout(Duration::from_secs(10), session_a.wait_for_idle())
        .await
        .unwrap();
    assert!(!session_a.is_streaming());
    assert_eq!(session_a.run_state(), RunState::Idle);
    assert_eq!(
        session_a
            .prompt("again".into(), PromptOptions::default())
            .await
            .unwrap(),
        PromptDisposition::Started
    );
    tokio::time::timeout(Duration::from_secs(10), session_a.wait_for_idle())
        .await
        .unwrap();
    assert_eq!(
        session_a.last_assistant_text().as_deref(),
        Some("recovered")
    );
    assert_eq!(provider_a.requests().len(), 2);

    // The sibling session still runs in the same process.
    let mut events_b = Events::new(&session_b);
    run_idle(&session_b, "sibling").await;
    assert_eq!(
        session_b.last_assistant_text().as_deref(),
        Some("sibling ok")
    );
    assert!(
        !events_b.has(is_session_crashed) && !events_b.has(is_extension_disabled),
        "the crash never touched session B's stream"
    );
}
