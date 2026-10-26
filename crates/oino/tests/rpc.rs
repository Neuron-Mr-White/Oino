//! `oino --mode rpc` over stdio with the faux provider: framing, every pi command (success
//! and error paths), events, the extension UI sub-protocol and the `_oino/*` extensions.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use std::time::Duration;

use serde_json::{Value, json};
use support::{Sandbox, kinds};

fn text(text: &str) -> Value {
    json!({"text": text})
}

#[test]
fn framing_parse_errors_unknown_commands_and_ids() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session"]);
    rpc.write_raw("not json\n");
    let parse = rpc.next();
    assert_eq!(parse["type"], "response");
    assert_eq!(parse["command"], "parse");
    assert_eq!(parse["success"], false);
    assert!(parse.get("id").is_none());
    assert!(
        parse["error"]
            .as_str()
            .unwrap()
            .starts_with("Failed to parse command: "),
        "{parse}"
    );
    rpc.write_raw("{\"id\":7,\"type\":\"nope\"}\r\n");
    assert_eq!(
        rpc.next(),
        json!({"id":7,"type":"response","command":"nope","success":false,"error":"Unknown command: nope"})
    );
    rpc.write_raw("{\"id\":\"x\"}\n");
    assert_eq!(
        rpc.next(),
        json!({"id":"x","type":"response","success":false,"error":"Unknown command: undefined"})
    );
    // U+2028 inside a string is payload, not a line break.
    rpc.write_raw("{\"id\":\"n\",\"type\":\"set_session_name\",\"name\":\"a\u{2028}b\"}\n");
    assert_eq!(rpc.next()["type"], "session_info_changed");
    assert_eq!(
        rpc.next(),
        json!({"id":"n","type":"response","command":"set_session_name","success":true})
    );
    let state = rpc.ok(json!({"type": "get_state"}));
    assert_eq!(state["sessionName"], "a\u{2028}b");
    // Responses without data have no `data` key.
    let abort = rpc.call(json!({"type": "abort"}));
    assert!(abort.get("data").is_none(), "{abort}");
    assert_eq!(rpc.close(), Some(0));
}

#[test]
fn prompt_streams_pi_events_after_its_response() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(
        &json!([
            {"thinking": "hmm", "toolCalls": [{"id": "c1", "name": "read", "arguments": {"path": "a.txt"}}]},
            text("done")
        ]),
        &["--no-session"],
    );
    std::fs::write(sandbox.cwd().join("a.txt"), "alpha\n").unwrap();
    let id = rpc.send(json!({"type": "prompt", "message": "read it"}));
    let (response, before) = rpc.until(|v| v["type"] == "response" && v["id"] == id);
    assert_eq!(response["data"], json!({"disposition": "started"}));
    assert!(
        before.iter().all(|e| e["type"] != "agent_start"),
        "{before:#?}"
    );
    let (_, events) = rpc.until(|v| v["type"] == "agent_settled");
    let kinds = kinds(&events);
    assert_eq!(kinds[0], "agent_start");
    for expected in [
        "turn_start",
        "message_start",
        "message_end",
        "tool_execution_start",
        "tool_execution_end",
        "turn_end",
        "agent_end",
    ] {
        assert!(
            kinds.contains(&expected.to_owned()),
            "{expected} in {kinds:?}"
        );
    }
    let updates: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "message_update")
        .collect();
    assert_ne!(updates.len(), 0);
    for update in &updates {
        let keys: Vec<&String> = update.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["type", "usage", "assistantMessageEvent"], "{update}");
        assert!(update["assistantMessageEvent"].get("partial").is_none());
    }
    let start = updates
        .iter()
        .find(|u| u["assistantMessageEvent"]["type"] == "toolcall_start")
        .unwrap();
    assert_eq!(start["assistantMessageEvent"]["id"], "c1");
    assert_eq!(start["assistantMessageEvent"]["toolName"], "read");
    assert!(
        updates
            .iter()
            .any(|u| u["assistantMessageEvent"]["type"] == "thinking_delta")
    );
    let end = events
        .iter()
        .find(|e| e["type"] == "tool_execution_end")
        .unwrap();
    assert_eq!(end["toolCallId"], "c1");
    assert_eq!(end["result"]["content"][0]["text"], "alpha\n");
    assert!(
        events
            .iter()
            .all(|e| e.get("id").is_none() || e["type"] == "response")
    );
    assert_eq!(
        rpc.ok(json!({"type": "get_last_assistant_text"})),
        json!({"text": "done"})
    );
    let messages = rpc.ok(json!({"type": "get_messages"}));
    let roles: Vec<&str> = messages["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert!(
        roles.ends_with(&["user", "assistant", "toolResult", "assistant"]),
        "{roles:?}"
    );
}

#[test]
fn state_model_and_thinking_commands() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session"]);
    let state = rpc.ok(json!({"type": "get_state"}));
    for key in [
        "thinkingLevel",
        "isStreaming",
        "isCompacting",
        "steeringMode",
        "followUpMode",
        "sessionId",
        "autoCompactionEnabled",
        "messageCount",
        "pendingMessageCount",
    ] {
        assert!(state.get(key).is_some(), "{key} missing in {state}");
    }
    assert_eq!(state["model"]["id"], "faux-1");
    assert!(
        state.get("sessionFile").is_none(),
        "--no-session has no file"
    );
    assert_eq!(state["isStreaming"], false);
    let models = rpc.ok(json!({"type": "get_available_models"}));
    assert_eq!(models["models"][0]["provider"], "faux");
    let model = rpc.ok(json!({"type": "set_model", "provider": "faux", "modelId": "faux-1"}));
    assert_eq!(model["id"], "faux-1");
    assert_eq!(
        rpc.err(json!({"type": "set_model", "provider": "faux", "modelId": "nope"})),
        "Model not found: faux/nope"
    );
    assert!(
        rpc.err(json!({"type": "set_model", "provider": "faux"}))
            .contains("modelId")
    );
    assert_eq!(rpc.ok(json!({"type": "cycle_model"})), Value::Null);
    let levels = rpc.ok(json!({"type": "get_available_thinking_levels"}));
    assert!(
        levels["levels"]
            .as_array()
            .unwrap()
            .contains(&json!("high"))
    );
    let (_, events) = rpc.ok_events(json!({"type": "set_thinking_level", "level": "high"}));
    assert_eq!(events[0]["type"], "thinking_level_changed");
    assert_eq!(events[0]["level"], "high");
    assert_eq!(
        rpc.ok(json!({"type": "get_state"}))["thinkingLevel"],
        "high"
    );
    let cycled = rpc.ok(json!({"type": "cycle_thinking_level"}));
    assert!(cycled["level"].is_string(), "{cycled}");
    rpc.ok(json!({"type": "set_steering_mode", "mode": "all"}));
    rpc.ok(json!({"type": "set_follow_up_mode", "mode": "all"}));
    rpc.ok(json!({"type": "set_auto_compaction", "enabled": false}));
    rpc.ok(json!({"type": "set_auto_retry", "enabled": false}));
    rpc.ok(json!({"type": "abort_retry"}));
    let state = rpc.ok(json!({"type": "get_state"}));
    assert_eq!(state["steeringMode"], "all");
    assert_eq!(state["followUpMode"], "all");
    assert_eq!(state["autoCompactionEnabled"], false);
    assert!(
        rpc.err(json!({"type": "set_steering_mode", "mode": "sometimes"}))
            .contains("unknown variant")
    );
}

#[test]
fn queueing_steering_follow_up_clear_and_abort() {
    let sandbox = Sandbox::new();
    let slow = json!({"text": "slow", "delayMs": 300});
    let mut rpc = sandbox.rpc(
        &json!([slow, text("steered"), text("followed"), {"text": "long", "delayMs": 2000}]),
        &["--no-session", "-e", "builtin:ui-probe"],
    );
    let id = rpc.send(json!({"type": "prompt", "message": "first"}));
    rpc.response(&id);
    rpc.event("agent_start");
    assert_eq!(
        rpc.err(json!({"type": "prompt", "message": "again"})),
        "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message."
    );
    assert_eq!(
        rpc.ok(json!({"type": "steer", "message": "s1"})),
        json!({"disposition": "queued"})
    );
    assert_eq!(
        rpc.ok(json!({"type": "prompt", "message": "f1", "streamingBehavior": "followUp"})),
        json!({"disposition": "queued"})
    );
    assert_eq!(
        rpc.ok(json!({"type": "get_state"}))["pendingMessageCount"],
        2
    );
    let (_, events) = rpc.until(|v| v["type"] == "agent_settled");
    let users: Vec<&Value> = events
        .iter()
        .filter(|e| e["type"] == "message_start" && e["message"]["role"] == "user")
        .map(|e| &e["message"]["content"][0]["text"])
        .collect();
    assert_eq!(users, [&json!("s1"), &json!("f1")]);
    assert!(rpc.seen.iter().any(|e| e["type"] == "queue_update"));

    // Clear while queued.
    let id = rpc.send(json!({"type": "prompt", "message": "long one"}));
    rpc.response(&id);
    rpc.event("agent_start");
    rpc.ok(json!({"type": "follow_up", "message": "later"}));
    assert_eq!(
        rpc.ok(json!({"type": "clear_queue"})),
        json!({"steering": [], "followUp": ["later"]})
    );
    assert!(
        rpc.err(json!({"type": "steer", "message": "/probe select"}))
            .contains("cannot be queued")
    );
    let (_, events) = rpc.ok_events(json!({"type": "abort"}));
    let settled = events
        .iter()
        .find(|e| e["type"] == "agent_settled")
        .unwrap();
    assert_eq!(settled["aborted"], true);
    assert_eq!(rpc.ok(json!({"type": "get_state"}))["isStreaming"], false);
}

#[test]
fn bash_streams_updates_with_the_request_id() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session"]);
    let id = rpc.send(json!({"type": "bash", "command": "printf 'one\\ntwo\\n'"}));
    let (response, before) = rpc.until(|v| v["type"] == "response" && v["id"] == id);
    let updates: String = before
        .iter()
        .filter(|e| e["type"] == "bash_execution_update")
        .map(|e| {
            assert_eq!(e["id"], id.as_str());
            e["delta"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(updates, "one\ntwo\n");
    assert_eq!(
        response["data"],
        json!({"output": "one\ntwo\n", "exitCode": 0, "cancelled": false, "truncated": false})
    );
    let excluded = rpc.ok(json!({"type": "bash", "command": "exit 3", "excludeFromContext": true}));
    assert_eq!(excluded["exitCode"], 3);
    let id = rpc.send(json!({"type": "bash", "command": "sleep 5"}));
    std::thread::sleep(Duration::from_millis(200));
    rpc.ok(json!({"type": "abort_bash"}));
    let cancelled = rpc.response(&id);
    assert_eq!(cancelled["data"]["cancelled"], true, "{cancelled}");
    let messages = rpc.ok(json!({"type": "get_messages"}));
    let bash: Vec<&Value> = messages["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "bashExecution")
        .collect();
    assert_eq!(bash.len(), 3);
    assert_eq!(bash[1]["excludeFromContext"], true);
}

#[test]
fn session_commands_entries_tree_fork_clone_switch_export_stats() {
    let sandbox = Sandbox::new();
    let sessions = sandbox.dir.path().join("sessions");
    let sessions_arg = sessions.display().to_string();
    let mut rpc = sandbox.rpc(
        &json!([text("one"), text("two"), text("three")]),
        &["--session-dir", &sessions_arg],
    );
    rpc.prompt_and_settle("first");
    rpc.prompt_and_settle("second");
    let state = rpc.ok(json!({"type": "get_state"}));
    let file = state["sessionFile"].as_str().unwrap().to_owned();
    assert!(std::path::Path::new(&file).exists());

    let entries = rpc.ok(json!({"type": "get_entries"}));
    let all = entries["entries"].as_array().unwrap().clone();
    assert_eq!(entries["leafId"], all.last().unwrap()["id"]);
    let since = rpc.ok(json!({"type": "get_entries", "since": all[0]["id"]}));
    assert_eq!(since["entries"].as_array().unwrap().len(), all.len() - 1);
    assert_eq!(
        rpc.err(json!({"type": "get_entries", "since": "zzzz"})),
        "Entry not found: zzzz"
    );
    let tree = rpc.ok(json!({"type": "get_tree"}));
    assert!(tree["tree"].is_array());
    assert_eq!(tree["leafId"], entries["leafId"]);

    let fork_messages = rpc.ok(json!({"type": "get_fork_messages"}));
    let forks = fork_messages["messages"].as_array().unwrap().clone();
    assert_eq!(
        forks.iter().map(|m| m["text"].clone()).collect::<Vec<_>>(),
        [json!("first"), json!("second")]
    );
    let fork = rpc.ok(json!({"type": "fork", "entryId": forks[1]["entryId"]}));
    assert_eq!(fork, json!({"text": "second", "cancelled": false}));
    let forked = rpc.ok(json!({"type": "get_state"}));
    assert_ne!(forked["sessionId"], state["sessionId"]);
    assert_eq!(
        rpc.err(json!({"type": "fork", "entryId": "nope"})),
        "Invalid entry ID for forking"
    );

    assert_eq!(
        rpc.ok(json!({"type": "clone"})),
        json!({"cancelled": false})
    );
    let cloned = rpc.ok(json!({"type": "get_state"}));
    assert_ne!(cloned["sessionId"], forked["sessionId"]);

    assert_eq!(
        rpc.ok(json!({"type": "switch_session", "sessionPath": file})),
        json!({"cancelled": false})
    );
    assert_eq!(
        rpc.ok(json!({"type": "get_state"}))["sessionId"],
        state["sessionId"]
    );

    assert_eq!(
        rpc.err(json!({"type": "set_session_name", "name": "   "})),
        "Session name cannot be empty"
    );
    let (_, events) = rpc.ok_events(json!({"type": "set_session_name", "name": " Named "}));
    assert_eq!(events.last().unwrap()["type"], "session_info_changed");
    assert_eq!(events.last().unwrap()["name"], "Named");
    assert_eq!(rpc.ok(json!({"type": "get_state"}))["sessionName"], "Named");

    let statistics = rpc.ok(json!({"type": "get_session_stats"}));
    assert_eq!(statistics["sessionId"], state["sessionId"]);
    assert_eq!(statistics["userMessages"], 2, "{statistics:#}");

    let html = sandbox.dir.path().join("out.html");
    let exported = rpc.ok(json!({"type": "export_html", "outputPath": html.display().to_string()}));
    assert_eq!(exported["path"], html.display().to_string());
    assert!(std::fs::read_to_string(&html).unwrap().contains("<html"));

    assert_eq!(
        rpc.ok(json!({"type": "new_session"})),
        json!({"cancelled": false})
    );
    let fresh = rpc.ok(json!({"type": "get_state"}));
    assert_ne!(fresh["sessionId"], state["sessionId"]);
    assert_eq!(fresh["messageCount"], 0);
    assert_eq!(
        rpc.ok(json!({"type": "get_last_assistant_text"})),
        json!({"text": null})
    );
    assert_eq!(rpc.close(), Some(0));
}

#[test]
fn compact_error_and_success_paths() {
    let sandbox = Sandbox::new();
    sandbox.write_settings(&json!({"compaction": {"keepRecentTokens": 1}}));
    let mut rpc = sandbox.rpc(
        &json!([text("one"), text("two"), text("## Goal\nSummary text")]),
        &["--no-session"],
    );
    assert_eq!(
        rpc.err(json!({"type": "compact"})),
        "Nothing to compact (session too small)"
    );
    rpc.prompt_and_settle("first question with some words");
    rpc.prompt_and_settle("second question with more words");
    let id = rpc.send(json!({"type": "compact", "customInstructions": "be brief"}));
    let (response, end) = rpc.response_and(&id, |v| v["type"] == "compaction_end");
    assert_eq!(end["reason"], "manual");
    let result = &response["data"];
    assert!(
        result["summary"].as_str().unwrap().contains("Summary text"),
        "{result:#}"
    );
    assert!(result["firstKeptEntryId"].is_string());
    assert!(result["tokensBefore"].is_number());
}

#[test]
fn get_commands_lists_extension_commands() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    let commands = rpc.ok(json!({"type": "get_commands"}));
    let probe = commands["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "probe")
        .cloned()
        .unwrap();
    assert_eq!(probe["source"], "extension");
    assert_eq!(probe["description"], "Raise an extension UI request");
    assert!(probe["sourceInfo"].is_object());
}

fn probe(rpc: &mut support::Rpc, args: &str) -> (String, Value) {
    let id = rpc.send(json!({"type": "prompt", "message": format!("/probe {args}")}));
    let request = rpc.event("extension_ui_request");
    (id, request)
}

/// The probe's report and the `handled` reply, in either order (they travel separately).
fn answer(rpc: &mut support::Rpc, id: &str) -> String {
    let mut notify = None;
    let mut handled = false;
    while notify.is_none() || !handled {
        let value = rpc.next();
        if value["type"] == "extension_ui_request" && value["method"] == "notify" {
            notify = value["message"].as_str().map(str::to_owned);
        } else if value["type"] == "response" && value["id"] == id {
            assert_eq!(value["data"], json!({"disposition": "handled"}));
            handled = true;
        }
    }
    notify.unwrap()
}

#[test]
fn extension_ui_dialogs_round_trip() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);

    let (id, request) = probe(&mut rpc, "select Pick one");
    assert_eq!(request["method"], "select");
    assert_eq!(request["title"], "Pick one");
    assert_eq!(request["options"], json!(["a", "b"]));
    assert!(
        request.get("extensionId").is_none(),
        "pi clients get pi records"
    );
    assert!(request["id"].as_str().unwrap().len() >= 32);
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "value": "b"})
    ));
    assert_eq!(answer(&mut rpc, &id), r#"probe:select:{"value":"b"}"#);

    let (id, request) = probe(&mut rpc, "confirm Sure");
    assert_eq!(request["message"], "Sure?");
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "confirmed": true})
    ));
    assert_eq!(answer(&mut rpc, &id), r#"probe:confirm:{"confirmed":true}"#);

    let (id, request) = probe(&mut rpc, "input Name");
    assert_eq!(request["placeholder"], "type");
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "cancelled": true})
    ));
    assert_eq!(answer(&mut rpc, &id), r#"probe:input:{"cancelled":true}"#);

    let (id, request) = probe(&mut rpc, "editor Edit");
    assert_eq!(request["prefill"], "draft");
    // Unknown ids are ignored; the real answer still wins.
    rpc.write_raw("{\"type\":\"extension_ui_response\",\"id\":\"nope\",\"value\":\"x\"}\n");
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "value": "edited"})
    ));
    assert_eq!(answer(&mut rpc, &id), r#"probe:editor:{"value":"edited"}"#);

    let (id, request) = probe(&mut rpc, "timeout Quick");
    assert_eq!(request["timeout"], 50);
    assert_eq!(answer(&mut rpc, &id), r#"probe:timeout:{"cancelled":true}"#);

    // Oino-only dialogs are resolved as cancelled for pi clients (never sent).
    let id = rpc.send(json!({"type": "prompt", "message": "/probe form"}));
    let seen = rpc.seen.len();
    assert_eq!(answer(&mut rpc, &id), r#"probe:form:{"cancelled":true}"#);
    assert!(rpc.seen[seen..].iter().all(|e| e["method"] != "form"));
}

#[test]
fn extension_ui_fire_and_forget_and_errors() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    let cases = [
        (
            "notify hello",
            json!({"method": "notify", "message": "hello", "notifyType": "warning"}),
        ),
        (
            "setStatus busy",
            json!({"method": "setStatus", "statusKey": "probe", "statusText": "busy"}),
        ),
        (
            "setWidget line",
            json!({"method": "setWidget", "widgetKey": "probe", "widgetLines": ["line"], "widgetPlacement": "belowEditor"}),
        ),
        ("setTitle T", json!({"method": "setTitle", "title": "T"})),
        (
            "set_editor_text draft",
            json!({"method": "set_editor_text", "text": "draft"}),
        ),
    ];
    for (args, expected) in cases {
        let id = rpc.send(json!({"type": "prompt", "message": format!("/probe {args}")}));
        let (_, request) = rpc.response_and(&id, |v| v["type"] == "extension_ui_request");
        let mut actual = request.clone();
        actual.as_object_mut().unwrap().remove("id");
        actual.as_object_mut().unwrap().remove("type");
        assert_eq!(actual, expected, "{args}");
    }
    // Oino fire-and-forget methods are dropped for pi clients.
    let id = rpc.send(json!({"type": "prompt", "message": "/probe setFrame x"}));
    rpc.response(&id);
    let id = rpc.send(json!({"type": "prompt", "message": "/probe-fail"}));
    let seen = rpc.seen.len();
    let (_, error) = rpc.response_and(&id, |v| v["type"] == "extension_error");
    assert!(rpc.seen[seen..].iter().all(|e| e["method"] != "setFrame"));
    // pi `_tryExecuteExtensionCommand` reports command failures as `command:<name>`.
    assert_eq!(error["extensionPath"], "command:probe-fail");
    assert_eq!(error["event"], "command");
    assert_eq!(error["error"], "probe failed");
    assert!(error.get("stack").is_none());
}

#[test]
fn oino_hello_unlocks_oino_events_and_commands() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(
        &json!([text("one"), text("two")]),
        &["--no-session", "-e", "builtin:ui-probe"],
    );
    let hello = rpc.ok(
        json!({"type": "_oino/hello", "client": {"name": "test", "version": "1"}, "protocol": 1}),
    );
    assert_eq!(hello["server"]["name"], "oino");
    assert_eq!(hello["protocol"], 1);
    let state = rpc.ok(json!({"type": "get_state"}));
    assert_eq!(hello["sessionId"], state["sessionId"]);

    let (id, request) = probe(&mut rpc, "form");
    assert_eq!(request["method"], "form");
    assert_eq!(request["extensionId"], "ui-probe");
    assert_eq!(request["sessionId"], state["sessionId"]);
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "value": {"answers": []}})
    ));
    let dismiss = rpc.event("extension_ui_dismiss");
    assert_eq!(dismiss["id"], request["id"]);
    assert_eq!(dismiss["reason"], "answered");
    assert_eq!(
        answer(&mut rpc, &id),
        r#"probe:form:{"value":{"answers":[]}}"#
    );

    let id = rpc.send(json!({"type": "prompt", "message": "/probe setFrame framed"}));
    let (_, frame) = rpc.response_and(&id, |v| v["type"] == "extension_ui_request");
    assert_eq!(frame["method"], "setFrame");
    let id = rpc.send(json!({"type": "prompt", "message": "/probe setStatus s"}));
    rpc.response_and(&id, |v| v["type"] == "extension_ui_request");
    let snapshot = rpc.ok(json!({"type": "_oino/ui_snapshot"}));
    let methods: Vec<&str> = snapshot["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["method"].as_str().unwrap())
        .collect();
    assert!(
        methods.contains(&"setFrame") && methods.contains(&"setStatus"),
        "{methods:?}"
    );

    rpc.prompt_and_settle("hi");
    let usage = rpc.ok(json!({"type": "_oino/get_context_usage"}));
    assert!(usage.is_object() || usage.is_null(), "{usage}");
    let entries = rpc.ok(json!({"type": "get_entries"}));
    let leaf = entries["leafId"].as_str().unwrap().to_owned();
    rpc.ok(json!({"type": "_oino/set_label", "entryId": leaf, "label": "mark"}));
    let user = entries["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "message" && e["message"]["role"] == "user")
        .unwrap()["id"]
        .clone();
    let navigated = rpc.ok(json!({"type": "_oino/navigate_tree", "targetId": user}));
    assert_eq!(navigated["editorText"], "hi");
    let jsonl = sandbox.dir.path().join("export.jsonl");
    let exported =
        rpc.ok(json!({"type": "_oino/export_jsonl", "outputPath": jsonl.display().to_string()}));
    assert_eq!(exported["path"], jsonl.display().to_string());
    let lines = support::jsonl(&jsonl);
    assert_eq!(lines[0]["type"], "session");
    assert_eq!(
        rpc.err(json!({"type": "_oino/import_jsonl", "path": "/nonexistent.jsonl"})),
        "File not found: /nonexistent.jsonl"
    );
    assert_eq!(
        rpc.err(json!({"type": "_oino/nope"})),
        "Unknown command: _oino/nope"
    );
}

#[test]
fn stdin_end_shuts_down_and_rejects_file_args() {
    let sandbox = Sandbox::new();
    let rpc = sandbox.rpc(&json!([]), &["--no-session"]);
    assert_eq!(rpc.close(), Some(0));
    let output = sandbox.run(&json!([]), &["--mode", "rpc", "@x.md"], None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "Error: @file arguments are not supported in RPC mode"
    );
}

#[test]
fn concurrent_commands_answer_while_a_dialog_blocks() {
    let sandbox = Sandbox::new();
    let mut rpc = sandbox.rpc(&json!([]), &["--no-session", "-e", "builtin:ui-probe"]);
    let (id, request) = probe(&mut rpc, "confirm Wait");
    // The pending command does not block other commands (pi handles lines concurrently).
    let state = rpc.ok(json!({"type": "get_state"}));
    assert_eq!(state["isStreaming"], false);
    rpc.write_raw(&format!(
        "{}\n",
        json!({"type": "extension_ui_response", "id": request["id"], "confirmed": false})
    ));
    assert_eq!(
        answer(&mut rpc, &id),
        r#"probe:confirm:{"confirmed":false}"#
    );
    assert!(rpc.try_next(Duration::from_millis(100)).is_none());
}
