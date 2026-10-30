//! Focus and key routing of a v2 entry (WP-U4, ui-protocol §4.6): Tab / Shift+Tab, arrows,
//! Enter, Esc, Space and entry `keys` inside a focused registry entry.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping; small JSON helpers take owned values"
)]

mod common;

use common::{Harness, TestHost};
use oino_tui::Effect;
use serde_json::{Value, json};

fn patch(rev: u64, entry: Value) -> Value {
    json!({"type": "oino_ui_patch", "scope": "session", "sessionId": "s-1", "rev": rev,
           "ops": [{"op": "put", "entry": entry}]})
}

fn entry(id: &str, slot: &str, view: Value) -> Value {
    json!({
        "id": id,
        "extensionId": "probe",
        "scope": "session",
        "slot": slot,
        "order": 0,
        "seq": 1,
        "view": view,
        "focus": true,
        "rev": 1
    })
}

fn delivered(h: &Harness) -> Vec<Value> {
    h.effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::UiAction {
                action_id,
                value,
                values,
                path,
                ..
            } => Some(json!({
                "action": action_id,
                "value": value,
                "values": values,
                "path": path
            })),
            _ => None,
        })
        .collect()
}

async fn open(view: Value) -> Harness {
    let mut h = Harness::new(TestHost::default()).await;
    h.json_event(patch(1, entry("probe/panel", "overlay", view)))
        .await;
    h.effects.clear();
    h
}

#[tokio::test]
async fn entry_keys_fire_actions() {
    let mut h = Harness::new(TestHost::default()).await;
    let mut entry = entry(
        "probe/panel",
        "overlay",
        json!({"kind": "tree", "root": {"t": "Paragraph", "text": "hello"}}),
    );
    entry["keys"] = json!([{"key": "r", "action": "refresh", "label": "Refresh"}]);
    entry["closeOnEscape"] = json!(false);
    h.json_event(patch(1, entry)).await;
    h.effects.clear();
    h.key("r").await;
    let acted = delivered(&h);
    assert_eq!(acted.len(), 1, "{acted:?}");
    assert_eq!(acted[0]["action"], "refresh");
    assert_eq!(acted[0]["path"], Value::Null);
    // Esc does not close when `closeOnEscape` is false.
    h.key("escape").await;
    assert_eq!(delivered(&h).len(), 1);
}

#[tokio::test]
async fn escape_closes_and_reports_values() {
    let mut h = open(json!({"kind": "tree", "root": {"t": "Input", "key": "name"}})).await;
    h.key("escape").await;
    let acted = delivered(&h);
    assert_eq!(acted[0]["action"], "$close");
}

#[tokio::test]
async fn tab_cycles_focus_and_enter_activates_buttons() {
    let mut h = open(json!({"kind": "tree", "root": {
        "t": "Layout",
        "constraints": [{"Length": 1}, {"Length": 1}],
        "children": [
            {"t": "Button", "label": "Go", "action": "go"},
            {"t": "Button", "label": "Stop", "action": "stop", "variant": "Danger"}
        ]
    }}))
    .await;
    h.key("enter").await;
    assert_eq!(delivered(&h)[0]["action"], "go");
    h.key("tab").await;
    h.key("enter").await;
    assert_eq!(delivered(&h)[1]["action"], "stop");
    h.key("tab").await;
    h.key("enter").await;
    assert_eq!(delivered(&h)[2]["action"], "go");
}

#[tokio::test]
async fn arrows_move_a_list_and_fire_on_select() {
    let mut h = open(json!({"kind": "tree", "root": {
        "t": "List",
        "items": [{"key": "a", "content": "A"}, {"key": "b", "content": "B"}],
        "onSelect": "pick",
        "onActivate": "open"
    }}))
    .await;
    h.key("down").await;
    let acted = delivered(&h);
    assert_eq!(acted[0]["action"], "pick");
    assert_eq!(acted[0]["value"], "b");
    h.key("enter").await;
    assert_eq!(delivered(&h)[1]["action"], "open");
    assert_eq!(delivered(&h)[1]["value"], "b");
    h.key("up").await;
    assert_eq!(delivered(&h)[2]["value"], "a");
}

#[tokio::test]
async fn tree_arrows_toggle_and_select() {
    let mut h = open(json!({"kind": "tree", "root": {
        "t": "Tree",
        "nodes": [{"key": "src", "label": "src", "children": [
            {"key": "lib", "label": "lib.rs"}
        ]}],
        "onSelect": "pick",
        "onActivate": "open",
        "onToggle": "fold"
    }}))
    .await;
    h.key("right").await;
    let acted = delivered(&h);
    assert_eq!(acted[0]["action"], "fold");
    assert_eq!(acted[0]["value"], json!({"key": "src", "expanded": true}));
    h.key("down").await;
    assert_eq!(delivered(&h)[1]["action"], "pick");
    assert_eq!(delivered(&h)[1]["value"], "lib");
    h.key("enter").await;
    assert_eq!(delivered(&h)[2]["action"], "open");
    h.key("up").await;
    h.key("left").await;
    assert_eq!(delivered(&h)[4]["action"], "fold");
    assert_eq!(
        delivered(&h)[4]["value"],
        json!({"key": "src", "expanded": false})
    );
}

#[tokio::test]
async fn input_submits_typed_value_with_collected_values() {
    let mut h = open(json!({"kind": "tree", "root": {
        "t": "Layout",
        "constraints": [{"Fill": 1}, {"Length": 1}, {"Length": 1}],
        "children": [
            {"t": "Input", "key": "name", "onSubmit": "save"},
            {"t": "Toggle", "key": "dry", "label": "Dry run", "value": true, "onChange": "dry"},
            {"t": "Select", "key": "env", "options": [{"value": "dev", "label": "dev"},
             {"value": "prod", "label": "prod"}], "onChange": "env"}
        ]
    }}))
    .await;
    h.type_text("app").await;
    h.key("enter").await;
    let acted = delivered(&h);
    assert_eq!(acted[0]["action"], "save");
    assert_eq!(acted[0]["value"], "app");
    assert_eq!(
        acted[0]["values"],
        json!({"name": "app", "dry": true, "env": null})
    );
    h.key("tab").await;
    h.key("space").await;
    let acted = delivered(&h);
    assert_eq!(acted[1]["action"], "dry");
    assert_eq!(acted[1]["value"], false);
    assert_eq!(acted[1]["values"]["dry"], json!(false));
    h.key("tab").await;
    h.key("space").await;
    h.key("space").await;
    let acted = delivered(&h);
    assert_eq!(acted[2]["action"], "env");
    assert_eq!(acted[2]["value"], "dev");
    assert_eq!(acted[3]["value"], "prod");
}

#[tokio::test]
async fn collapsible_block_toggles_locally() {
    let mut h = open(json!({"kind": "tree", "root": {
        "t": "Block",
        "title": "Logs",
        "borders": "ALL",
        "collapsible": true,
        "onToggle": "fold",
        "child": {"t": "Paragraph", "text": "line"}
    }}))
    .await;
    h.key("enter").await;
    let acted = delivered(&h);
    assert_eq!(acted[0]["action"], "fold");
    assert_eq!(acted[0]["value"], true);
    h.key("enter").await;
    assert_eq!(delivered(&h)[1]["value"], false);
}
