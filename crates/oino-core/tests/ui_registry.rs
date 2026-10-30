//! Session-level registry wiring (ui-protocol §6, §9): producers publish patches, turn-footer
//! entries come from renderer registrations, visibility aggregates to `$show`/`$hide`, a
//! `$close` disposes after delivery, invalid ids become `extension_error`s.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use oino_ai::JsonObject;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::Registration;
use oino_core::hub::EventStream;
use oino_core::session::{AgentSession, RenderRequest, RenderTarget};
use oino_core::testing::{FakeExtension, FauxProvider, TestSession};
use oino_core::ui::UiAction;
use oino_ui::slot::{RegistryOp, Slot, UiSnapshot, UiView};
use serde_json::{Value, json};

fn args(value: Value) -> JsonObject {
    serde_json::from_value(value).unwrap()
}

/// An extension that registers a turn-footer renderer and records its UI events.
fn extension(events: &Arc<Mutex<Vec<(String, Value)>>>) -> Arc<FakeExtension> {
    let registration = Registration {
        renderers: vec![json!({"target": "turn-footer", "key": "files"})],
        ..Registration::default()
    };
    let recorded = Arc::clone(events);
    Arc::new(
        FakeExtension::new("probe", registration)
            .on_event(move |kind, payload| {
                recorded.lock().unwrap().push((kind.to_owned(), payload));
                Ok(None)
            })
            .on_render(&[], Some),
    )
}

async fn session(events: Arc<Mutex<Vec<(String, Value)>>>) -> AgentSession {
    TestSession {
        extensions: vec![extension(&events)],
        has_ui: true,
        ..TestSession::default()
    }
    .build(FauxProvider::new(Vec::new()))
    .await
}

/// The next registry patch.
async fn next_patch(events: &mut EventStream) -> (u64, Vec<RegistryOp>) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("timed out waiting for a patch")
            .expect("stream ended");
        if let ClientEvent::Oino(OinoEvent::UiPatch { rev, ops, .. }) = event {
            return (rev, ops);
        }
    }
}

async fn wait_until(session: &AgentSession, check: impl Fn(&UiSnapshot) -> bool) {
    for _ in 0..200 {
        if check(&session.ui_snapshot()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one scenario per acceptance row of WP-U3"
)]
async fn producers_publish_patches_and_turn_footers_dispatch() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let session = session(Arc::clone(&events)).await;

    // The static renderer registration produced its turn-footer entry (§6.7).
    let snapshot = session.ui_snapshot();
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.id == "probe/turnFooter:files")
        .expect("no turn-footer entry");
    assert_eq!(entry.slot, Slot::TurnFooter);
    assert_eq!(
        entry.view,
        UiView::Renderer {
            key: "files".into()
        }
    );

    // A retained producer publishes one patch with rev = previous + 1 (§9.2).
    let mut stream = session.subscribe();
    session
        .ui_fire(
            "probe",
            "setStatus",
            args(json!({"statusKey": "k", "statusText": "1"})),
        )
        .unwrap();
    let (rev, ops) = next_patch(&mut stream).await;
    assert_eq!(rev, snapshot.rev + 1);
    let RegistryOp::Put { entry } = &ops[0] else {
        panic!("expected a put, got {:?}", ops[0]);
    };
    assert_eq!(entry.id, "probe/status:k");
    assert_eq!(entry.slot, Slot::Status);
    assert_eq!(session.ui_snapshot().rev, rev);

    // A foreign id is rejected and published as `extension_error` (§9.1).
    let error = session
        .ui_fire(
            "probe",
            "openPanel",
            args(json!({"panelId": "other/panel-1", "placement": "sidePane"})),
        )
        .unwrap_err();
    assert!(error.to_string().contains("not owned"));

    // The turn-footer render dispatch carries the turn identity (§6.7).
    let node = session
        .render_spec(&RenderRequest {
            target: RenderTarget::TurnFooter,
            key: "files".into(),
            turn_entry_id: Some("t1".into()),
            turn_index: Some(2),
            running: true,
            ..RenderRequest::default()
        })
        .unwrap()
        .expect("no render result");
    assert_eq!(
        node,
        json!({"target": "turnFooter", "key": "files", "turnEntryId": "t1",
               "turnIndex": 2, "running": true})
    );

    // `invalidateTurnFooter` bumps the entry and emits an invalidate op.
    session
        .ui_fire(
            "probe",
            "invalidateTurnFooter",
            args(json!({"key": "files", "turnEntryIds": ["t1"]})),
        )
        .unwrap();
    let (_, ops) = next_patch(&mut stream).await;
    assert_eq!(
        ops,
        [RegistryOp::Invalidate {
            id: "probe/turnFooter:files".into(),
            turns: Some(vec!["t1".into()])
        }]
    );

    // Visibility aggregates over frontends (§6.5): the owner gets `$show`/`$hide`.
    session.ui_visible("tui", "probe/turnFooter:files", true);
    session.ui_visible("tui", "probe/turnFooter:files", false);
    wait_until(&session, |snapshot| snapshot.rev > 0).await;
    let actions: Vec<String> = {
        for _ in 0..200 {
            let seen: Vec<String> = events
                .lock()
                .unwrap()
                .iter()
                .filter(|(kind, _)| kind == "oino_ui_action")
                .map(|(_, payload)| payload["actionId"].as_str().unwrap_or("").to_owned())
                .collect();
            if seen.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _)| kind == "oino_ui_action")
            .map(|(_, payload)| payload["actionId"].as_str().unwrap_or("").to_owned())
            .collect()
    };
    assert_eq!(actions, ["$show", "$hide"]);

    // A user `$close` is delivered first, then deletes the entry (§6.6).
    session
        .ui_action_with(&UiAction {
            panel_id: "probe/status:k".into(),
            action_id: "$close".into(),
            ..UiAction::default()
        })
        .unwrap();
    wait_until(&session, |snapshot| {
        snapshot
            .entries
            .iter()
            .all(|entry| entry.id != "probe/status:k")
    })
    .await;
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, payload)| payload["actionId"] == "$close")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispose_clears_the_registry() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let session = session(events).await;
    session
        .ui_fire(
            "probe",
            "openPanel",
            args(json!({"placement": "sidePane", "spec": {"t": "Paragraph", "text": "hi"}})),
        )
        .unwrap();
    assert_eq!(
        session.ui_snapshot().entries.len(),
        2,
        "turn footer + panel"
    );
    session.dispose().await;
    assert_eq!(session.ui_snapshot().entries.len(), 0);
}
