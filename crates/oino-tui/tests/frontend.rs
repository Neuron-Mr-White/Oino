//! Backend-driven frontend features: merged completions, editor intents, the extension
//! settings hub and provider sign-in (modes.md §4, ui-protocol §6.6, §7).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use std::time::Duration;

use common::{Harness, TestHost};
use oino_tui::AppEvent;
use serde_json::{Value, json};

async fn harness() -> Harness {
    Harness::new(TestHost::default()).await
}

fn sent(h: &Harness, kind: &str) -> Vec<Value> {
    h.commands()
        .into_iter()
        .filter(|c| c["type"] == kind)
        .collect()
}

fn schema() -> Value {
    json!({"namespaces": [{
        "namespace": "notify", "title": "Notify", "extensionId": "unipi.notify",
        "sections": [{"id": "events", "title": "Events", "fields": [
            {"key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "default": true},
            {"key": "platform", "type": "enum", "label": "Platform", "default": "native",
             "options": [{"value": "native", "label": "Native"}, {"value": "ntfy", "label": "ntfy"}]},
            {"key": "ntfy.token", "type": "secret", "label": "ntfy token", "scope": "global",
             "description": "Token for the ntfy server"},
            {"key": "test", "type": "action", "label": "Send test", "command": "unipi:notify-test"}
        ]}]
    }]})
}

#[tokio::test]
async fn argument_completions_come_from_the_backend() {
    let mut h = harness().await;
    h.fake.reply(
        "_oino/get_completions",
        Ok(Some(
            json!({"groups": [{"source": "command:probe", "from": 7, "items": [
                {"value": "select", "description": "pick one"}, {"value": "setStatus"}
            ]}]}),
        )),
    );
    h.type_text("/probe se").await;
    let request = sent(&h, "_oino/get_completions").pop().unwrap();
    assert_eq!(
        request,
        json!({"type": "_oino/get_completions", "text": "/probe se", "cursor": 9})
    );
    let frame = h.frame(70, 20);
    assert!(
        frame.contains("select") && frame.contains("pick one"),
        "{frame}"
    );
    h.key("tab").await;
    h.key("enter").await;
    let prompt = sent(&h, "prompt").pop().unwrap();
    assert_eq!(prompt["message"], "/probe select");
}

#[tokio::test]
async fn editor_intents_are_reported() {
    let mut h = harness().await;
    h.key("down").await;
    let consumable = sent(&h, "_oino/editor_event").pop().unwrap();
    assert_eq!(consumable["intent"], "emptyArrowDown");
    assert_eq!(consumable["text"], "");
    h.type_text("draft").await;
    tokio::time::sleep(Duration::from_millis(170)).await;
    h.app.handle(AppEvent::Tick);
    h.pump().await;
    let changed = sent(&h, "_oino/editor_event").pop().unwrap();
    assert_eq!(changed["intent"], "changed");
    assert_eq!(changed["text"], "draft");
    h.key("enter").await;
    let submit = sent(&h, "_oino/editor_event").pop().unwrap();
    assert_eq!(submit["intent"], "submit");
    assert_eq!(submit["text"], "draft");
}

#[tokio::test]
async fn settings_hub_lists_and_edits_extension_namespaces() {
    let mut h = harness().await;
    h.fake
        .reply("_oino/get_settings_schema", Ok(Some(schema())));
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(json!({"values": {"events": {"agentEnd": true}, "platform": "ntfy", "ntfy": {"token": "••••••"}}}))),
    );
    h.type_text("/settings").await;
    h.key("enter").await;
    // The namespace row comes after the core rows; `up` wraps to it.
    h.key("up").await;
    insta::assert_snapshot!("settings_hub_rows", h.frame(80, 30));
    h.key("enter").await;
    insta::assert_snapshot!("settings_hub_namespace", h.frame(80, 18));
    assert_eq!(
        sent(&h, "_oino/get_extension_settings")[0]["namespace"],
        "notify"
    );
    h.key("enter").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(
        set,
        json!({"type": "_oino/set_extension_setting", "namespace": "notify", "key": "events.agentEnd", "value": false, "scope": "global"})
    );
    h.key("down").await;
    h.key("tab").await;
    h.key("left").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["value"], "native");
    assert_eq!(set["scope"], "project");
    h.key("delete").await;
    let unset = sent(&h, "_oino/unset_extension_setting").pop().unwrap();
    assert_eq!(unset["key"], "platform");
    h.key("down").await;
    h.key("enter").await;
    h.type_text("tok").await;
    h.key("enter").await;
    let secret = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(secret["key"], "ntfy.token");
    assert_eq!(secret["value"], "tok");
    assert_eq!(secret["scope"], "global", "global-only field");
    h.key("escape").await;
    let frame = h.frame(80, 30);
    assert!(frame.contains(" Settings"), "back to /settings: {frame}");
}

#[tokio::test]
async fn open_settings_for_a_namespace_opens_its_page() {
    let mut h = harness().await;
    h.fake
        .reply("_oino/get_settings_schema", Ok(Some(schema())));
    h.json_event(
        json!({"type": "extension_ui_request", "id": "o1", "method": "openSettings",
        "namespace": "notify", "extensionId": "unipi.notify"}),
    )
    .await;
    let frame = h.frame(80, 18);
    assert!(frame.contains("Settings › Notify"), "{frame}");
}

#[tokio::test]
async fn login_and_logout_pick_a_provider() {
    let mut h = harness().await;
    h.fake.reply(
        "_oino/auth_providers",
        Ok(Some(json!({"providers": [
            {"id": "anthropic", "name": "Anthropic", "oauth": true, "configured": false, "env": ["ANTHROPIC_API_KEY"]},
            {"id": "openai", "name": "OpenAI", "oauth": true, "configured": true, "stored": "oauth", "env": []},
            {"id": "groq", "name": "Groq", "oauth": false, "configured": false, "env": ["GROQ_API_KEY"]}
        ]}))),
    );
    h.type_text("/login").await;
    h.key("enter").await;
    insta::assert_snapshot!("login_picker", h.frame(70, 16));
    h.key("enter").await;
    assert_eq!(
        sent(&h, "_oino/login").pop().unwrap(),
        json!({"type": "_oino/login", "provider": "anthropic"})
    );
    assert!(h.frame(70, 16).contains("Signed in to anthropic"));
    h.type_text("/logout").await;
    h.key("enter").await;
    h.key("enter").await;
    assert_eq!(
        sent(&h, "_oino/logout").pop().unwrap(),
        json!({"type": "_oino/logout", "provider": "openai"})
    );
    h.type_text("/login groq").await;
    h.key("enter").await;
    assert_eq!(sent(&h, "_oino/login").pop().unwrap()["provider"], "groq");
}

fn line_text(lines: &[ratatui::text::Line<'_>]) -> String {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn regular_mode_moves_finished_rows_to_the_scrollback() {
    let host = TestHost::default();
    host.settings
        .lock()
        .unwrap()
        .insert("tuiMode".into(), json!("regular"));
    let mut h = Harness::new(host).await;
    assert!(h.app.inline());
    let header = oino_tui::render::take_scrollback(&mut h.app, 80);
    assert!(line_text(&header).contains("oino v1.0.0-test"));
    assert_eq!(oino_tui::render::take_scrollback(&mut h.app, 80).len(), 0);
    assert!(
        !h.frame(80, 14).contains("oino v1.0.0-test"),
        "header left the viewport"
    );

    let user = json!({"role": "user", "content": "hello there", "timestamp": 0});
    h.json_event(json!({"type": "agent_start"})).await;
    h.json_event(json!({"type": "message_start", "message": user}))
        .await;
    h.json_event(json!({"type": "message_end", "message": user}))
        .await;
    assert_eq!(
        oino_tui::render::take_scrollback(&mut h.app, 80).len(),
        0,
        "nothing is committed while the turn runs"
    );
    assert!(h.frame(80, 14).contains("hello there"));
    h.json_event(json!({"type": "agent_end", "messages": [], "willRetry": false}))
        .await;
    h.json_event(json!({"type": "agent_settled", "aborted": false}))
        .await;
    let committed = oino_tui::render::take_scrollback(&mut h.app, 80);
    assert!(
        line_text(&committed).contains("hello there"),
        "{committed:?}"
    );
    assert!(!h.frame(80, 14).contains("hello there"));
}
