//! Behaviour tests: key presses and slash commands → backend commands, dialog answers,
//! session replacement, keybinding overrides (pi interactive-mode semantics).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use common::{Harness, TestHost};
use oino_tui::Effect;
use oino_tui::host::KeybindingOverrides;
use serde_json::json;

async fn harness() -> Harness {
    Harness::new(TestHost::default()).await
}

fn last(h: &Harness, kind: &str) -> serde_json::Value {
    h.commands()
        .into_iter()
        .rfind(|c| c["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} command in {:?}", h.command_types()))
}

#[tokio::test]
async fn startup_requests_session_state() {
    let h = harness().await;
    let types = h.command_types();
    for kind in [
        "_oino/hello",
        "_oino/ui_snapshot",
        "get_state",
        "get_entries",
        "get_commands",
        "get_available_models",
        "get_available_thinking_levels",
        "get_session_stats",
        "_oino/get_context_usage",
    ] {
        assert!(types.iter().any(|t| t == kind), "missing {kind}: {types:?}");
    }
}

#[tokio::test]
async fn prompts_steer_and_follow_up() {
    let mut h = harness().await;
    h.type_text("hello").await;
    h.key("enter").await;
    assert_eq!(last(&h, "prompt")["message"], "hello");
    assert!(last(&h, "prompt").get("streamingBehavior").is_none());
    h.json_event(json!({"type": "agent_start"})).await;
    h.type_text("also this").await;
    h.key("enter").await;
    assert_eq!(last(&h, "prompt")["streamingBehavior"], "steer");
    h.type_text("later").await;
    h.key("alt+enter").await;
    assert_eq!(last(&h, "follow_up")["message"], "later");
    h.key("escape").await;
    assert_eq!(h.command_types().last().unwrap(), "abort");
}

#[tokio::test]
async fn ctrl_c_clears_then_quits() {
    let mut h = harness().await;
    h.type_text("draft").await;
    h.key("ctrl+c").await;
    assert!(!h.app.should_quit());
    h.key("ctrl+c").await;
    assert!(h.app.should_quit());
}

#[tokio::test]
async fn ctrl_d_exits_only_when_empty() {
    let mut h = harness().await;
    h.type_text("x").await;
    h.key("ctrl+d").await;
    assert!(!h.app.should_quit());
    h.key("ctrl+u").await;
    h.key("ctrl+d").await;
    assert!(h.app.should_quit());
}

#[tokio::test]
async fn slash_commands_map_to_session_commands() {
    let mut h = harness().await;
    for (text, kind) in [
        ("/model faux-2", "set_model"),
        ("/thinking high", "set_thinking_level"),
        ("/compact keep the API notes", "compact"),
        ("/name refactor", "set_session_name"),
        ("/export out.jsonl", "_oino/export_jsonl"),
        ("/export", "export_html"),
        ("/clone", "clone"),
        ("/new", "new_session"),
        ("/import old.jsonl", "_oino/import_jsonl"),
        ("/session", "get_session_stats"),
        ("/tree", "get_tree"),
        ("/fork", "get_fork_messages"),
    ] {
        h.type_text(text).await;
        h.key("escape").await; // close the slash menu
        h.key("enter").await;
        assert!(
            h.command_types().iter().any(|t| t == kind),
            "{text} → {kind}"
        );
        h.app
            .handle(oino_tui::AppEvent::Key("escape".parse().unwrap()));
        h.pump().await;
    }
    assert_eq!(last(&h, "set_model")["modelId"], "faux-2");
    assert_eq!(last(&h, "set_thinking_level")["level"], "high");
    assert_eq!(
        last(&h, "compact")["customInstructions"],
        "keep the API notes"
    );
    assert_eq!(last(&h, "set_session_name")["name"], "refactor");
}

#[tokio::test]
async fn bash_commands_route_to_bash() {
    let mut h = harness().await;
    h.type_text("!!git status").await;
    h.key("enter").await;
    let bash = last(&h, "bash");
    assert_eq!(bash["command"], "git status");
    assert_eq!(bash["excludeFromContext"], true);
}

#[tokio::test]
async fn fork_picker_restores_text() {
    let mut h = harness().await;
    h.fake.reply(
        "get_fork_messages",
        Ok(Some(json!({"messages": [{"entryId": "u1", "text": "first"}, {"entryId": "u2", "text": "second"}]}))),
    );
    h.fake.reply(
        "fork",
        Ok(Some(json!({"text": "second", "cancelled": false}))),
    );
    h.type_text("/fork").await;
    h.key("enter").await;
    h.key("enter").await;
    assert_eq!(last(&h, "fork")["entryId"], "u2");
    let frame = h.frame(60, 16);
    assert!(frame.contains("second"), "{frame}");
}

#[tokio::test]
async fn session_replacement_resets_and_refetches() {
    let mut h = harness().await;
    h.type_text("hi").await;
    h.key("enter").await;
    let before = h.fake.commands().len();
    h.json_event(json!({"type": "session_replaced", "sessionId": "s-2", "reason": "new"}))
        .await;
    let after: Vec<String> = h.command_types()[before..].to_vec();
    assert!(after.contains(&"get_entries".to_owned()));
    assert!(after.contains(&"get_state".to_owned()));
}

#[tokio::test]
async fn keybinding_overrides_apply() {
    let mut overrides = KeybindingOverrides::default();
    overrides
        .actions
        .insert("app.session.new".into(), vec!["ctrl+shift+n".into()]);
    overrides
        .actions
        .insert("app.thinking.cycle".into(), vec![]);
    let mut h = Harness::new(TestHost {
        keybindings: overrides,
        ..TestHost::default()
    })
    .await;
    h.key("ctrl+shift+n").await;
    assert_eq!(h.command_types().last().unwrap(), "new_session");
    let before = h.fake.commands().len();
    h.key("shift+tab").await;
    assert_eq!(
        h.fake.commands().len(),
        before,
        "unbound action must not fire"
    );
}

#[tokio::test]
async fn model_and_thinking_cycling() {
    let mut h = harness().await;
    h.key("ctrl+p").await;
    assert_eq!(h.command_types().last().unwrap(), "cycle_model");
    h.key("shift+tab").await;
    assert_eq!(h.command_types().last().unwrap(), "cycle_thinking_level");
}

#[tokio::test]
async fn history_and_completion() {
    let mut h = harness().await;
    h.type_text("first prompt").await;
    h.key("enter").await;
    h.key("up").await;
    assert!(h.frame(60, 14).contains("first prompt"));
    h.key("ctrl+u").await;
    h.type_text("look at @sr").await;
    h.key("tab").await;
    let frame = h.frame(60, 14);
    assert!(frame.contains("@src/"), "{frame}");
}

#[tokio::test]
async fn dialogs_dismissed_by_core_close() {
    let mut h = harness().await;
    h.json_event(json!({"type": "extension_ui_request", "id": "x", "method": "confirm", "title": "Sure?", "message": ""}))
        .await;
    assert!(h.frame(60, 14).contains("Sure?"));
    h.json_event(json!({"type": "extension_ui_dismiss", "id": "x", "reason": "answered"}))
        .await;
    assert!(!h.frame(60, 14).contains("Sure?"));
}

#[tokio::test]
async fn editor_text_and_notify_effects() {
    let mut h = harness().await;
    h.json_event(json!({"type": "extension_ui_request", "id": "a", "method": "set_editor_text", "text": "prefilled"})).await;
    h.json_event(json!({"type": "extension_ui_request", "id": "b", "method": "notify", "message": "heads up", "notifyType": "warning"})).await;
    h.json_event(json!({"type": "extension_ui_request", "id": "c", "method": "clipboardWrite", "text": "copied"})).await;
    let frame = h.frame(60, 16);
    assert!(
        frame.contains("prefilled") && frame.contains("heads up"),
        "{frame}"
    );
    assert!(
        h.effects
            .iter()
            .any(|e| matches!(e, Effect::ClipboardWrite(t) if t == "copied"))
    );
}

#[tokio::test]
async fn extension_shortcuts_invoke() {
    let mut h = harness().await;
    h.json_event(json!({"type": "extension_ui_request", "id": "s", "method": "setShortcuts", "extensionId": "probe",
        "shortcuts": [{"id": "probe.go", "keys": ["alt+h"], "description": "Go", "extensionId": "probe"},
                      {"id": "probe.steal", "keys": ["ctrl+p"], "description": "Steal", "extensionId": "probe"}]}))
        .await;
    h.key("alt+h").await;
    assert!(
        h.effects
            .iter()
            .any(|e| matches!(e, Effect::InvokeShortcut(id) if id == "probe.go"))
    );
    h.key("ctrl+p").await;
    assert!(
        !h.effects
            .iter()
            .any(|e| matches!(e, Effect::InvokeShortcut(id) if id == "probe.steal"))
    );
}

#[tokio::test]
async fn initial_messages_after_load() {
    let host = std::sync::Arc::new(TestHost::default());
    let mut app = oino_tui::App::new(
        host,
        oino_tui::TuiOptions {
            version: "t".into(),
            initial_messages: vec!["from cli".into()],
            ..oino_tui::TuiOptions::default()
        },
    );
    app.start();
    let mut h = Harness {
        app,
        fake: common::backend(),
        host: std::sync::Arc::new(TestHost::default()),
        effects: Vec::new(),
    };
    h.pump().await;
    let types = h.command_types();
    let entries = types.iter().position(|t| t == "get_entries").unwrap();
    let prompt = types.iter().position(|t| t == "prompt").unwrap();
    assert!(prompt > entries);
    assert_eq!(last(&h, "prompt")["message"], "from cli");
}

#[tokio::test]
async fn transcript_rebuilds_from_entries_with_custom_renderer() {
    let host = TestHost {
        render_reply: Some(
            json!({"t": "card", "title": "Probe entry", "rail": "accent", "body": {"t": "text", "value": "n = 1"}}),
        ),
        ..TestHost::default()
    };
    let mut h = Harness::new(host).await;
    h.fake.reply(
        "get_entries",
        Ok(Some(json!({"leafId": "b", "entries": [
            {"type": "message", "id": "a", "parentId": null, "timestamp": "2026-01-01T00:00:00.000Z",
             "message": {"role": "user", "content": "hi", "timestamp": 0}},
            {"type": "custom", "id": "b", "parentId": "a", "timestamp": "2026-01-01T00:00:01.000Z", "customType": "probe", "data": {"n": 1}}
        ]}))),
    );
    h.json_event(json!({"type": "session_replaced", "sessionId": "s-1", "reason": "resume"}))
        .await;
    let frame = h.frame(60, 20);
    assert!(
        frame.contains("Probe entry") && frame.contains("n = 1"),
        "{frame}"
    );
    let renders = h.host.renders.lock().unwrap().clone();
    assert!(
        renders
            .iter()
            .any(|r| r.target == "customEntry" && r.key == "probe")
    );
}
