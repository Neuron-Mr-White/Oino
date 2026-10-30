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
            {"key": "test", "type": "action", "label": "Send test", "command": "notify-test"}
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

fn oss2_schema() -> Value {
    json!({"namespaces": [{
        "namespace": "unipi.mcp", "title": "MCP", "extensionId": "unipi.mcp",
        "icon": "gear", "order": 5,
        "sections": [{"id": "servers", "title": "Servers", "fields": [
            {"key": "servers", "type": "list", "label": "Servers", "addLabel": "Add server",
             "item": {"type": "string", "label": "URL"}},
            {"key": "env", "type": "dict", "label": "Environment", "keyLabel": "Name",
             "value": {"type": "string", "label": "Value"}},
            {"key": "raw", "type": "json", "label": "Raw config"},
            {"key": "mode", "type": "enum", "label": "Mode", "default": "simple",
             "options": [{"value": "simple", "label": "Simple"}, {"value": "custom", "label": "Custom"}]},
            {"key": "customCommand", "type": "string", "label": "Custom command",
             "visibleIf": {"key": "mode", "eq": "custom"},
             "disabledIf": {"key": "locked", "truthy": true},
             "restart": "session", "description": "Runs for every turn"},
            {"key": "neverShown", "type": "string", "label": "Never shown",
             "visibleIf": {"key": "mode", "eq": "never"}}
        ]}]
    }]})
}

fn oss2_values(servers: &Value, env: &Value) -> Value {
    json!({
        "values": {
            "servers": servers,
            "env": env,
            "raw": {"a": 1},
            "mode": "custom",
            "locked": true
        },
        "diagnostics": []
    })
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one scenario per OSS-2 field type")]
async fn oss2_fields_render_and_edit_in_sub_pages() {
    let mut h = harness().await;
    h.fake
        .reply("_oino/get_settings_schema", Ok(Some(oss2_schema())));
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(json!({
            "values": {
                "servers": ["https://a"],
                "env": {"TOKEN": "x"},
                "raw": {"a": 1},
                "mode": "custom",
                "locked": true
            },
            "diagnostics": [{"namespace": "unipi.mcp", "key": "raw", "message": "expected an object"}]
        }))),
    );
    h.type_text("/settings").await;
    h.key("enter").await;
    // The namespace row comes after the core rows; `up` wraps to it.
    h.key("up").await;
    h.key("enter").await;
    let frame = h.frame(80, 24);
    assert!(
        frame.contains("1 item(s)") && frame.contains("1 key(s)"),
        "{frame}"
    );
    assert!(
        frame.contains("ignored: raw"),
        "diagnostics are shown: {frame}"
    );
    assert!(
        frame.contains("Custom command") && !frame.contains("Never shown"),
        "visibleIf gates the rows: {frame}"
    );
    insta::assert_snapshot!("settings_hub_oss2", h.frame(80, 24));

    // `list`: enter opens the row sub-page, `a` appends a row, editing writes the array.
    h.key("enter").await;
    let frame = h.frame(80, 24);
    assert!(
        frame.contains("Servers › rows") && frame.contains("https://a"),
        "{frame}"
    );
    insta::assert_snapshot!("settings_hub_sub_page", frame);
    // The reload after each write returns the stored value (a real backend resolves it).
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(oss2_values(
            &json!(["https://a", ""]),
            &json!({"TOKEN": "x"}),
        ))),
    );
    h.key("a").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["key"], "servers");
    assert_eq!(set["value"], json!(["https://a", ""]));
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(oss2_values(
            &json!(["https://a", "https://b"]),
            &json!({"TOKEN": "x"}),
        ))),
    );
    h.key("enter").await;
    h.type_text("https://b").await;
    h.key("enter").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["value"], json!(["https://a", "https://b"]));
    // `del` removes the selected row.
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(oss2_values(
            &json!(["https://a"]),
            &json!({"TOKEN": "x"}),
        ))),
    );
    h.key("delete").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["value"], json!(["https://a"]));
    h.key("escape").await;

    // `dict`: rows are keyed and a new row gets a fresh key.
    h.key("down").await;
    h.key("enter").await;
    h.fake.reply(
        "_oino/get_extension_settings",
        Ok(Some(oss2_values(
            &json!(["https://a"]),
            &json!({"TOKEN": "x", "new": ""}),
        ))),
    );
    h.key("a").await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["key"], "env");
    assert_eq!(set["value"], json!({"TOKEN": "x", "new": ""}));
    h.key("escape").await;

    // `json`: opened in the external editor (`editExternally`), written back as JSON.
    h.key("down").await;
    h.key("enter").await;
    assert!(
        h.effects.iter().any(|effect| matches!(
            effect,
            oino_tui::Effect::ExternalEditor {
                purpose: oino_tui::app::EditPurpose::Setting { key, .. },
                ..
            } if key == "raw"
        )),
        "the json field opens the external editor"
    );
    h.app.handle(AppEvent::Edited(
        oino_tui::app::EditPurpose::Setting {
            namespace: "unipi.mcp".into(),
            key: "raw".into(),
        },
        Some("{\"b\": 2}".into()),
    ));
    h.pump().await;
    let set = sent(&h, "_oino/set_extension_setting").pop().unwrap();
    assert_eq!(set["key"], "raw");
    assert_eq!(set["value"], json!({"b": 2}));

    // A `disabledIf` row renders but rejects edits.
    h.key("down").await;
    h.key("down").await;
    h.key("enter").await;
    let frame = h.frame(80, 24);
    assert!(frame.contains("is disabled here"), "{frame}");
    assert!(
        frame.contains("applies after the session restarts"),
        "{frame}"
    );
}
