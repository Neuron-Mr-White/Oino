//! The `_oino/*` frontend command surface over `oino --mode rpc` (modes.md §4): completions,
//! panel actions, shortcuts, editor intents, extension and core settings, service calls and
//! provider sign-in, driven against `builtin:ui-probe` and the faux provider.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use serde_json::{Value, json};
use support::Sandbox;

fn hello(rpc: &mut support::Rpc) {
    rpc.ok(
        json!({"type": "_oino/hello", "client": {"name": "test", "version": "1"}, "protocol": 1}),
    );
}

/// The next `notify` message.
fn notified(rpc: &mut support::Rpc) -> String {
    let (value, _) = rpc.until(|v| v["type"] == "extension_ui_request" && v["method"] == "notify");
    value["message"].as_str().unwrap().to_owned()
}

fn values(group: &Value) -> Vec<String> {
    group["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["value"].as_str().unwrap().to_owned())
        .collect()
}

fn group<'a>(groups: &'a Value, source: &str) -> &'a Value {
    groups["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["source"] == source)
        .unwrap_or_else(|| panic!("no {source} group in {groups:#}"))
}

#[test]
fn completions_cover_commands_arguments_paths_and_contributors() {
    let sandbox = Sandbox::new();
    std::fs::create_dir(sandbox.cwd().join("src")).unwrap();
    std::fs::write(sandbox.cwd().join("src/main.rs"), "").unwrap();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);

    let commands = rpc.ok(json!({"type": "_oino/get_completions", "text": "/pro", "cursor": 4}));
    let names = group(&commands, "commands");
    assert_eq!(names["from"], 0);
    assert!(values(names).contains(&"/probe".to_owned()), "{names}");

    let args = rpc.ok(json!({"type": "_oino/get_completions", "text": "/probe se", "cursor": 9}));
    let args = group(&args, "command:probe");
    assert_eq!(args["from"], 7);
    assert_eq!(
        values(args),
        [
            "select",
            "setStatus",
            "setWidget",
            "setTitle",
            "set_editor_text",
            "setFrame"
        ]
    );

    let paths =
        rpc.ok(json!({"type": "_oino/get_completions", "text": "see @src/ma", "cursor": 11}));
    let files = group(&paths, "files");
    assert_eq!(files["from"], 4);
    assert_eq!(values(files)[0], "@src/main.rs");

    let tags = rpc.ok(json!({"type": "_oino/get_completions", "text": "x #p", "cursor": 4}));
    let probe = group(&tags, "probe");
    assert_eq!(probe["from"], 2);
    assert_eq!(values(probe), ["#probe"]);

    let none = rpc.ok(json!({"type": "_oino/get_completions", "text": "plain", "cursor": 5}));
    assert_eq!(none, json!({"groups": []}));
    assert_eq!(rpc.close(), Some(0));
}

#[test]
fn panel_actions_shortcuts_and_editor_events_reach_the_extension() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    hello(&mut rpc);
    let id = rpc.send(json!({"type": "prompt", "message": "/probe panel Board"}));
    let (_, panel) = rpc.response_and(&id, |v| v["method"] == "openPanel");
    assert_eq!(panel["panelId"], "ui-probe/panel-0");

    rpc.ok(json!({"type": "_oino/ui_action", "panelId": "ui-probe/panel-0", "actionId": "go", "value": "x"}));
    assert_eq!(notified(&mut rpc), r#"probe:action:go:"x""#);
    assert_eq!(
        rpc.err(json!({"type": "_oino/ui_action", "panelId": "nope", "actionId": "go"})),
        "Unknown panel: nope"
    );

    let shortcuts = rpc.ok(json!({"type": "_oino/get_shortcuts"}));
    assert_eq!(
        shortcuts,
        json!({"shortcuts": [{"id": "probe.ping", "defaultKeys": ["ctrl+alt+p"], "description": "Ping the probe", "extensionId": "ui-probe"}]})
    );
    rpc.ok(json!({"type": "_oino/invoke_shortcut", "shortcutId": "probe.ping"}));
    assert_eq!(notified(&mut rpc), "probe:shortcut:probe.ping");
    assert_eq!(
        rpc.err(json!({"type": "_oino/invoke_shortcut", "shortcutId": "nope"})),
        "Unknown shortcut: nope"
    );

    let consumed = rpc.ok(json!({"type": "_oino/editor_event", "eventId": "e1", "intent": "emptyArrowDown", "text": ""}));
    assert_eq!(consumed, json!({"consumed": true}));
    let changed = rpc.ok(json!({"type": "_oino/editor_event", "eventId": "e2", "intent": "changed", "text": "draft"}));
    assert_eq!(changed, json!({"consumed": false}));
    let id = rpc.send(json!({"type": "prompt", "message": "/probe editor-text"}));
    let (_, report) = rpc.response_and(&id, |v| v["method"] == "notify");
    assert_eq!(report["message"], r#"probe:editor-text:"draft""#);
    rpc.ok(
        json!({"type": "_oino/editor_event", "eventId": "e3", "intent": "submit", "text": "go"}),
    );
    assert_eq!(notified(&mut rpc), "probe:editor:submit:go");
    assert!(
        rpc.err(
            json!({"type": "_oino/editor_event", "eventId": "e4", "intent": "nope", "text": ""})
        )
        .contains("unknown variant")
    );
    assert_eq!(rpc.close(), Some(0));
}

#[test]
fn extension_and_core_settings_round_trip() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    let schema = rpc.ok(json!({"type": "_oino/get_settings_schema"}));
    let probe = schema["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["namespace"] == "probe")
        .unwrap();
    assert_eq!(probe["extensionId"], "ui-probe");

    let defaults = rpc.ok(json!({"type": "_oino/get_extension_settings", "namespace": "probe"}));
    assert_eq!(defaults["values"]["enabled"], true);
    assert_eq!(defaults["values"]["mode"], "a");

    rpc.ok(json!({"type": "_oino/set_extension_setting", "namespace": "probe", "key": "mode", "value": "b", "scope": "global"}));
    rpc.ok(json!({"type": "_oino/set_extension_setting", "namespace": "probe", "key": "token", "value": "s3cret", "scope": "global"}));
    let global = rpc.ok(
        json!({"type": "_oino/get_extension_settings", "namespace": "probe", "scope": "global"}),
    );
    assert_eq!(global["values"]["mode"], "b");
    assert_ne!(global["values"]["token"], "s3cret", "secrets are masked");
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.agent().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(file["extensionSettings"]["probe"]["mode"], "b", "{file:#}");
    assert!(!file.to_string().contains("s3cret"));

    rpc.ok(json!({"type": "_oino/unset_extension_setting", "namespace": "probe", "key": "mode", "scope": "global"}));
    let reset = rpc.ok(json!({"type": "_oino/get_extension_settings", "namespace": "probe"}));
    assert_eq!(reset["values"]["mode"], "a");
    assert_eq!(
        rpc.err(json!({"type": "_oino/get_extension_settings", "namespace": "nope"})),
        "Unknown settings namespace: nope"
    );

    rpc.ok(
        json!({"type": "_oino/set_setting", "key": "tui.inline", "value": true, "scope": "global"}),
    );
    let value = rpc.ok(json!({"type": "_oino/get_settings", "key": "tui.inline"}));
    assert_eq!(value, json!({"value": true}));
    let all = rpc.ok(json!({"type": "_oino/get_settings"}));
    assert_eq!(all["settings"]["tui"]["inline"], true);
    assert_eq!(rpc.close(), Some(0));
}

#[test]
fn service_calls_and_provider_sign_in() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    hello(&mut rpc);
    let echo =
        rpc.ok(json!({"type": "_oino/service_call", "name": "probe.echo", "payload": {"n": 1}}));
    assert_eq!(echo, json!({"echo": {"n": 1}, "caller": "oino.frontend"}));
    assert_eq!(
        rpc.err(json!({"type": "_oino/service_call", "target": "faux", "name": "probe.echo"})),
        "Unknown service: faux/probe.echo"
    );

    let providers = rpc.ok(json!({"type": "_oino/auth_providers"}));
    let oauth = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "faux-oauth")
        .cloned()
        .unwrap();
    assert_eq!(oauth["oauth"], true);
    assert_eq!(oauth["configured"], false);
    assert_eq!(
        rpc.err(json!({"type": "_oino/login", "provider": "faux"})),
        "Faux does not support OAuth sign-in; set an API key instead"
    );

    let id = rpc.send(json!({"type": "_oino/login", "provider": "faux-oauth"}));
    let dialog = rpc.event("extension_ui_request");
    assert_eq!(dialog["method"], "input");
    assert_eq!(dialog["title"], "Sign in to faux-oauth");
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": dialog["id"], "value": "code-123"})
    ));
    let response = rpc.response(&id);
    assert_eq!(response["success"], true, "{response:#}");
    let auth: Value =
        serde_json::from_str(&std::fs::read_to_string(sandbox.agent().join("auth.json")).unwrap())
            .unwrap();
    assert_eq!(auth["faux-oauth"]["type"], "oauth");
    assert_eq!(auth["faux-oauth"]["access"], "code-123");
    let models = rpc.ok(json!({"type": "get_available_models"}));
    assert!(
        models["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["provider"] == "faux-oauth"),
        "{models:#}"
    );

    rpc.ok(json!({"type": "_oino/logout", "provider": "faux-oauth"}));
    let auth: Value =
        serde_json::from_str(&std::fs::read_to_string(sandbox.agent().join("auth.json")).unwrap())
            .unwrap();
    assert!(auth.get("faux-oauth").is_none());
    rpc.ok(json!({"type": "_oino/set_api_key", "provider": "faux", "key": "k2"}));
    let auth: Value =
        serde_json::from_str(&std::fs::read_to_string(sandbox.agent().join("auth.json")).unwrap())
            .unwrap();
    assert_eq!(auth["faux"], json!({"type": "api_key", "key": "k2"}));
    assert_eq!(rpc.close(), Some(0));
}
