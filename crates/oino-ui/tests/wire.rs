//! Registry wire types (§6.1, §9.2, §4.7): serde round-trips against the shared
//! `fixtures/registry/*.json` corpus and the §4.6 action example.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]
use oino_ui::prelude::*;
use oino_ui::slot::{
    ActionSource, DismissReason, KeyBinding, PanelOpen, PanelUpdate, RegistryOp, Scope, Slot,
    UiAction, UiCapabilities, UiPatch, UiSnapshot,
};
use serde_json::Value as Json;

#[test]
fn snapshot_fixture_round_trips() {
    let raw: Json = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/registry/snapshot.json"
    )))
    .unwrap_or_default();
    let snapshot: UiSnapshot = serde_json::from_value(raw.clone()).unwrap_or_default();
    assert_eq!(snapshot.scope, Scope::Session);
    assert_eq!(snapshot.entries.len(), 3);
    assert_eq!(snapshot.entries[0].slot, Slot::Fullscreen);
    assert_eq!(snapshot.dialogs.len(), 1);
    assert_eq!(snapshot.meta.working_message.as_deref(), Some("running"));
    let back = serde_json::to_value(&snapshot).unwrap_or_default();
    let again: UiSnapshot = serde_json::from_value(back).expect("fixture must parse");
    assert_eq!(snapshot, again);
}

#[test]
fn patch_fixture_round_trips() {
    let raw: Json = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/registry/patch.json"
    )))
    .unwrap_or_default();
    let patch: UiPatch = serde_json::from_value(raw.clone()).unwrap_or_default();
    assert_eq!(patch.rev, 5);
    assert_eq!(patch.ops.len(), 9);
    let mut saw_tree = false;
    for op in &patch.ops {
        if let RegistryOp::Tree { ops, .. } = op {
            saw_tree = true;
            assert_eq!(ops.len(), 5);
        }
    }
    assert!(saw_tree, "patch fixture keeps a tree op");
    let back = serde_json::to_value(&patch).unwrap_or_default();
    let again: UiPatch = serde_json::from_value(back).expect("fixture must parse");
    assert_eq!(patch, again);
}

#[test]
fn action_event_parses_spec_example() {
    let raw = r#"{"type":"oino_ui_action","panelId":"acme.board/board","actionId":"open-task",
        "value":"T-12","path":["main","tasks"],"values":{"filter":"open"},"source":"app"}"#;
    let value: Json = serde_json::from_str(raw).unwrap_or_default();
    let action: UiAction = serde_json::from_value(value.clone()).unwrap_or_default();
    assert_eq!(action.panel_id, "acme.board/board");
    assert_eq!(action.action_id, "open-task");
    assert_eq!(action.value.as_ref().and_then(Json::as_str), Some("T-12"));
    assert_eq!(
        action.path,
        Some(vec!["main".to_owned(), "tasks".to_owned()])
    );
    assert_eq!(action.source, Some(ActionSource::App));
    assert_eq!(oino_ui::slot::UI_ACTION_EVENT_TYPE, "oino_ui_action");
    // Without the envelope "type", the payload alone parses too.
    let payload: UiAction = serde_json::from_value(
        value
            .as_object()
            .map(|o| {
                let mut o = o.clone();
                o.remove("type");
                Json::Object(o)
            })
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    assert_eq!(payload, action);
}

#[test]
fn panel_open_and_update_round_trip() {
    let open = PanelOpen {
        panel_id: Some("tasks".into()),
        placement: Slot::SidePane,
        title: Some("Tasks".into()),
        icon: Some(Icon::List),
        order: Some(10),
        spec: Some(Node::List(List::new(["a"]))),
        keys: vec![KeyBinding {
            key: "enter".into(),
            action: Some("open".into()),
            label: "open".into(),
        }],
        focus: Some(true),
        close_on_escape: Some(true),
        ..PanelOpen::default()
    };
    let json = serde_json::to_value(&open).unwrap_or_default();
    let back: PanelOpen = serde_json::from_value(json).expect("fixture must parse");
    assert_eq!(open, back);

    let update = PanelUpdate {
        patch: Some(vec![NodeOp::Append {
            path: Vec::new(),
            field: PatchField::Lines,
            items: vec![PatchItem::Item(Json::String("line".into()))],
        }]),
        title: Some("Tasks".into()),
        ..PanelUpdate::default()
    };
    let json = serde_json::to_value(&update).unwrap_or_default();
    let back: PanelUpdate = serde_json::from_value(json).expect("fixture must parse");
    assert_eq!(update, back);
}

#[test]
fn capabilities_match_section_10() {
    let caps = UiCapabilities::full();
    let json = serde_json::to_value(&caps).unwrap_or_default();
    assert_eq!(json["protocol"], 2);
    assert_eq!(json["nodes"], "all");
    assert_eq!(json["slots"].as_array().map(Vec::len), Some(12));
    assert_eq!(json["reserved"].as_array().map(Vec::len), Some(9));
    let dialogs = json["dialogs"].as_array().cloned().unwrap_or_default();
    assert!(dialogs.contains(&Json::String("clipboardReadImage".into())));
    // The TUI declares no reserved components.
    let tui = UiCapabilities {
        reserved: Vec::new(),
        ..UiCapabilities::full()
    };
    let json = serde_json::to_value(&tui).unwrap_or_default();
    assert_eq!(
        json["reserved"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .len(),
        0
    );
}

#[test]
fn dismiss_reasons_and_order() {
    let reasons = [
        DismissReason::Answered,
        DismissReason::Timeout,
        DismissReason::Cancelled,
    ];
    for reason in reasons {
        let json = serde_json::to_value(reason).unwrap_or_default();
        let back: DismissReason = serde_json::from_value(json).expect("fixture must parse");
        assert_eq!(reason, back);
    }
}
