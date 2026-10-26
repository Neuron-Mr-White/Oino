//! `insta` snapshots of whole frames rendered with ratatui's `TestBackend` (modes.md §7.3).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use common::{Harness, TestHost, model};
use oino_view::sessions::SessionItem;
use serde_json::{Value, json};

fn usage() -> Value {
    json!({"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 15,
           "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}})
}

fn assistant(content: &Value, stop: &str) -> Value {
    json!({"role": "assistant", "content": content, "api": "faux", "provider": "faux", "model": "faux-1",
           "usage": usage(), "stopReason": stop, "timestamp": 0})
}

fn ui(id: &str, method: &str, params: &Value) -> Value {
    let mut v =
        json!({"type": "extension_ui_request", "id": id, "method": method, "extensionId": "probe"});
    for (k, val) in params.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

async fn harness() -> Harness {
    Harness::new(TestHost::default()).await
}

#[tokio::test]
async fn startup_screen() {
    let mut h = harness().await;
    insta::assert_snapshot!(h.frame(80, 24));
}

#[tokio::test]
async fn streaming_reply_with_tools() {
    let mut h = harness().await;
    let user = json!({"role": "user", "content": "Fix the bug in main.rs", "timestamp": 0});
    h.json_event(json!({"type": "agent_start"})).await;
    h.json_event(json!({"type": "message_start", "message": user}))
        .await;
    h.json_event(json!({"type": "message_end", "message": user}))
        .await;
    let partial = assistant(
        &json!([{"type": "thinking", "thinking": "The bug is an off-by-one."},
                {"type": "text", "text": "Let me **read** the file."},
                {"type": "toolCall", "id": "t1", "name": "read", "arguments": {"path": "src/main.rs"}},
                {"type": "toolCall", "id": "t2", "name": "bash", "arguments": {"command": "cargo test"}},
                {"type": "toolCall", "id": "t3", "name": "edit", "arguments": {"path": "src/main.rs"}}]),
        "toolUse",
    );
    h.json_event(json!({"type": "message_start", "message": assistant(&json!([]), "pending")}))
        .await;
    h.json_event(json!({"type": "message_update", "message": partial,
        "assistantMessageEvent": {"type": "start", "partial": partial}}))
        .await;
    h.json_event(json!({"type": "message_end", "message": partial}))
        .await;
    h.json_event(json!({"type": "tool_execution_start", "toolCallId": "t1", "toolName": "read", "args": {"path": "src/main.rs"}})).await;
    h.json_event(json!({"type": "tool_execution_end", "toolCallId": "t1", "toolName": "read",
        "result": {"content": [{"type": "text", "text": "fn main() {\n    println!(\"hi\");\n}"}]}, "isError": false})).await;
    h.json_event(json!({"type": "tool_execution_start", "toolCallId": "t2", "toolName": "bash", "args": {"command": "cargo test"}})).await;
    h.json_event(json!({"type": "tool_execution_update", "toolCallId": "t2", "toolName": "bash", "args": {"command": "cargo test"},
        "partialResult": {"content": [{"type": "text", "text": "running 3 tests\ntest a ... ok"}]}})).await;
    h.json_event(json!({"type": "tool_execution_start", "toolCallId": "t3", "toolName": "edit", "args": {"path": "src/main.rs"}})).await;
    h.json_event(json!({"type": "tool_execution_end", "toolCallId": "t3", "toolName": "edit",
        "result": {"content": [{"type": "text", "text": "ok"}], "details": {"diff": "-1 let x = 0;\n+1 let x = 1;"}}, "isError": false})).await;
    insta::assert_snapshot!(h.frame(80, 48));
    h.key("ctrl+o").await;
    h.key("ctrl+t").await;
    insta::assert_snapshot!("streaming_reply_expanded", h.frame(80, 60));
}

#[tokio::test]
async fn errors_custom_messages_and_summaries() {
    let mut h = harness().await;
    h.json_event(json!({"type": "message_start", "message": {"role": "custom", "customType": "notify", "content": "Build **finished**", "display": true, "timestamp": 0}})).await;
    h.json_event(json!({"type": "message_start", "message": assistant(&json!([]), "pending")}))
        .await;
    let failed = json!({"role": "assistant", "content": [], "api": "faux", "provider": "faux", "model": "faux-1",
        "usage": usage(), "stopReason": "error", "errorMessage": "rate limited", "timestamp": 0});
    h.json_event(json!({"type": "message_end", "message": failed}))
        .await;
    h.json_event(json!({"type": "auto_retry_end", "success": false, "attempt": 3, "finalError": "rate limited"})).await;
    h.json_event(json!({"type": "extension_error", "extensionPath": "unipi.notify", "event": "agent_end", "error": "boom"})).await;
    insta::assert_snapshot!(h.frame(80, 30));
}

#[tokio::test]
async fn bash_command_row() {
    let mut h = harness().await;
    h.fake.reply("bash", Ok(Some(json!({"output": "a.txt\nb.txt\n", "exitCode": 0, "cancelled": false, "truncated": false}))));
    h.type_text("!ls").await;
    h.key("enter").await;
    insta::assert_snapshot!(h.frame(60, 20));
    assert_eq!(h.commands().last().unwrap()["command"], "ls");
}

#[tokio::test]
async fn dialog_select_and_confirm() {
    let mut h = harness().await;
    h.json_event(ui(
        "d1",
        "select",
        &json!({"title": "Pick a color", "options": ["red", "green", "blue"],
        "optionDetails": [{"label": "red", "description": "warm"}]}),
    ))
    .await;
    h.key("down").await;
    insta::assert_snapshot!("dialog_select", h.frame(60, 20));
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[0]).unwrap()["value"],
        "green"
    );
    h.json_event(ui(
        "d2",
        "confirm",
        &json!({"title": "Delete file?", "message": "This cannot be undone."}),
    ))
    .await;
    insta::assert_snapshot!("dialog_confirm", h.frame(60, 20));
    h.key("n").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[1]).unwrap()["confirmed"],
        false
    );
}

#[tokio::test]
async fn dialog_input_and_editor() {
    let mut h = harness().await;
    h.json_event(ui(
        "i1",
        "input",
        &json!({"title": "Your name", "placeholder": "type here"}),
    ))
    .await;
    insta::assert_snapshot!("dialog_input_empty", h.frame(60, 16));
    h.type_text("Ada").await;
    insta::assert_snapshot!("dialog_input", h.frame(60, 16));
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[0]).unwrap()["value"],
        "Ada"
    );
    h.json_event(ui(
        "e1",
        "editor",
        &json!({"title": "Commit message", "prefill": "fix: bug\n\nDetails"}),
    ))
    .await;
    insta::assert_snapshot!("dialog_editor", h.frame(60, 18));
    h.key("escape").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[1]).unwrap()["cancelled"],
        true
    );
}

#[tokio::test]
async fn dialog_multiselect_form_approval() {
    let mut h = harness().await;
    h.json_event(ui("m1", "multiSelect", &json!({"title": "Features", "options": [
        {"label": "Tests", "value": "tests", "selected": true}, {"label": "Docs", "description": "markdown"}, {"label": "Bench"}]}))).await;
    h.key("down").await;
    h.key("space").await;
    insta::assert_snapshot!("dialog_multiselect", h.frame(60, 16));
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[0]).unwrap()["value"],
        json!(["tests", "Docs"])
    );

    h.json_event(ui("f1", "form", &json!({"title": "Questions", "questions": [
        {"id": "q1", "header": "Scope", "question": "What should I change?", "options": [{"label": "Only tests"}, {"label": "Everything", "description": "big"}], "other": true},
        {"id": "q2", "header": "Style", "question": "Pick styles", "multiSelect": true, "options": [{"label": "Terse"}, {"label": "Verbose"}]}
    ]}))).await;
    insta::assert_snapshot!("dialog_form", h.frame(60, 20));
    h.key("2").await;
    h.key("space").await;
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[1]).unwrap()["value"],
        json!({"answers": [{"id": "q1", "selected": ["Everything"]}, {"id": "q2", "selected": ["Terse"]}]})
    );

    h.json_event(ui("a1", "approval", &json!({"title": "Allow bash?", "toolName": "bash", "summary": "rm -rf target",
        "detail": {"t": "code", "value": "rm -rf target", "language": "sh"},
        "options": [{"optionId": "y", "label": "Allow once", "kind": "allow_once"}, {"optionId": "n", "label": "Reject", "kind": "reject_once"}],
        "feedback": {"placeholder": "Why?"}}))).await;
    insta::assert_snapshot!("dialog_approval", h.frame(60, 20));
    h.key("down").await;
    h.key("tab").await;
    h.type_text("unsafe").await;
    h.key("tab").await;
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[2]).unwrap()["value"],
        json!({"optionId": "n", "feedback": "unsafe"})
    );
}

#[tokio::test]
async fn dialog_custom_ous() {
    let mut h = harness().await;
    h.json_event(ui("c1", "custom", &json!({"title": "Deploy", "spec": {"t": "stack", "children": [
        {"t": "kv", "items": [{"key": "Env", "value": "prod"}, {"key": "Version", "value": [{"text": "1.2.3", "tone": "accent"}]}]},
        {"t": "toggle", "id": "dry", "label": "Dry run", "value": true, "onChange": "dry"},
        {"t": "meter", "value": 3, "max": 4, "label": "Checks"},
        {"t": "button", "label": "Deploy", "actionId": "deploy", "primary": true},
        {"t": "button", "label": "Cancel", "actionId": "cancel"}
    ]}}))).await;
    insta::assert_snapshot!(h.frame(60, 20));
    h.key("tab").await;
    h.key("enter").await;
    assert_eq!(
        serde_json::to_value(&h.fake.ui_responses()[0]).unwrap()["value"],
        json!({"actionId": "deploy"})
    );
}

fn panel(placement: &str) -> Value {
    ui(
        &format!("p-{placement}"),
        "openPanel",
        &json!({"panelId": "probe/dash", "placement": placement, "title": "Dashboard",
        "spec": {"t": "stack", "children": [
            {"t": "list", "selectable": "pick", "items": [{"id": "a", "label": "Alpha", "badge": "new"}, {"id": "b", "label": "Beta", "detail": "second"}]},
            {"t": "sparkline", "values": [1, 3, 2, 5, 4], "label": "load"},
            {"t": "keyHints", "hints": [{"keys": "r", "label": "refresh"}]}]},
        "keys": [{"key": "r", "actionId": "refresh", "label": "Refresh"}]}),
    )
}

#[tokio::test]
async fn panels_by_placement() {
    for placement in [
        "overlay",
        "sidePane",
        "editorReplace",
        "bottomPane",
        "fullscreen",
    ] {
        let mut h = harness().await;
        h.json_event(panel(placement)).await;
        insta::assert_snapshot!(format!("panel_{placement}"), h.frame(70, 22));
    }
}

#[tokio::test]
async fn widgets_status_frame_footer() {
    let mut h = harness().await;
    h.json_event(ui(
        "w1",
        "setWidget",
        &json!({"widgetKey": "todo", "widgetLines": ["☐ write tests", "☑ ship"]}),
    ))
    .await;
    h.json_event(ui(
        "w2",
        "setWidget",
        &json!({"widgetKey": "below", "widgetPlacement": "belowEditor",
        "widgetSpec": {"t": "badge", "value": "work tray", "tone": "success"}}),
    ))
    .await;
    h.json_event(ui(
        "s1",
        "setStatus",
        &json!({"statusKey": "jobs", "statusText": "2 jobs", "tone": "warning"}),
    ))
    .await;
    h.json_event(ui(
        "f1",
        "setFrame",
        &json!({"frameKey": "f", "top": [{"text": "plan mode", "tone": "accent", "priority": 2}],
        "bottom": [{"text": "ctx 12%", "align": "right", "priority": 1}]}),
    ))
    .await;
    h.json_event(ui(
        "h1",
        "setHeader",
        &json!({"key": "splash", "spec": {"t": "text", "value": "Welcome back"}}),
    ))
    .await;
    h.json_event(ui(
        "t1",
        "setWorkingMessage",
        &json!({"message": "Thinking hard..."}),
    ))
    .await;
    h.json_event(json!({"type": "agent_start"})).await;
    h.json_event(json!({"type": "queue_update", "steering": ["also check docs"], "followUp": []}))
        .await;
    h.type_text("draft").await;
    insta::assert_snapshot!(h.frame(80, 30));
}

#[tokio::test]
async fn slash_menu_and_model_selector() {
    let mut h = harness().await;
    h.type_text("/re").await;
    insta::assert_snapshot!("slash_menu", h.frame(70, 22));
    h.key("escape").await;
    h.key("ctrl+u").await;
    h.key("ctrl+l").await;
    insta::assert_snapshot!("model_selector", h.frame(70, 22));
    h.key("down").await;
    h.key("enter").await;
    let set = h
        .commands()
        .into_iter()
        .rfind(|c| c["type"] == "set_model")
        .unwrap();
    assert_eq!(set["modelId"], "faux-2");
}

#[tokio::test]
async fn settings_screen() {
    let mut h = harness().await;
    h.type_text("/settings").await;
    h.key("enter").await;
    insta::assert_snapshot!(h.frame(80, 26));
    h.key("enter").await;
    assert!(
        h.commands()
            .iter()
            .any(|c| c["type"] == "set_auto_compaction" && c["enabled"] == false)
    );
    assert_eq!(h.host.settings.lock().unwrap()["compaction.enabled"], false);
}

fn entry(id: &str, parent: Option<&str>, message: &Value) -> Value {
    json!({"type": "message", "id": id, "parentId": parent, "timestamp": "2026-01-01T00:00:00.000Z", "message": message})
}

#[tokio::test]
async fn tree_navigator() {
    let mut h = harness().await;
    let user = |t: &str| json!({"role": "user", "content": t, "timestamp": 0});
    let reply = assistant(&json!([{"type": "text", "text": "Done."}]), "stop");
    h.fake.reply("get_tree", Ok(Some(json!({"leafId": "d", "tree": [
        {"entry": entry("a", None, &user("start")), "children": [
            {"entry": entry("b", Some("a"), &reply), "children": [
                {"entry": entry("c", Some("b"), &user("try plan A")), "children": [
                    {"entry": entry("d", Some("c"), &reply), "children": []}]},
                {"entry": entry("e", Some("b"), &user("try plan B")), "children": [], "label": "alt"}]}]}]}))));
    h.type_text("/tree").await;
    h.key("enter").await;
    insta::assert_snapshot!("tree_navigator", h.frame(70, 22));
    h.key("down").await;
    h.key("enter").await;
    insta::assert_snapshot!("tree_summary_prompt", h.frame(70, 22));
    h.key("enter").await;
    let nav = h
        .commands()
        .into_iter()
        .rfind(|c| c["type"] == "_oino/navigate_tree")
        .unwrap();
    assert_eq!(nav["targetId"], "e");
    assert_eq!(nav["summarize"], false);
}

#[tokio::test]
async fn session_picker() {
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let item = |path: &str, name: Option<&str>, parent: Option<&str>, hours: i64| SessionItem {
        path: path.into(),
        id: path.into(),
        cwd: "/work/project".into(),
        name: name.map(str::to_owned),
        parent_path: parent.map(str::to_owned),
        modified_ms: now - hours * 3_600_000,
        message_count: 4,
        first_message: format!("first message of {path}"),
        all_text: String::new(),
    };
    let host = TestHost {
        sessions: vec![
            item("/s/a.jsonl", None, None, 2),
            item("/s/b.jsonl", Some("refactor"), Some("/s/a.jsonl"), 1),
            item("/s/c.jsonl", None, None, 30),
        ],
        ..TestHost::default()
    };
    let mut h = Harness::new(host).await;
    h.type_text("/resume").await;
    h.key("enter").await;
    insta::assert_snapshot!(h.frame(80, 20));
    h.key("down").await;
    h.key("enter").await;
    let switch = h
        .commands()
        .into_iter()
        .rfind(|c| c["type"] == "switch_session")
        .unwrap();
    assert_eq!(switch["sessionPath"], "/s/b.jsonl");
    let _ = model();
}
