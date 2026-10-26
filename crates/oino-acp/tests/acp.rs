//! ACP v1 integration tests: a scripted JSON-RPC client drives `oino-acp` over in-memory
//! pipes against real `oino-core` sessions with the faux provider.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    missing_docs,
    reason = "integration tests assert by unwrapping"
)]

mod support;

use oino_ai::{AssistantContent, StopReason, Usage};
use oino_core::testing::FauxResponse;
use serde_json::{Value as JsonValue, json};
use support::{ClientPolicy, Fixture, tool_call};

fn fs_caps() -> JsonValue {
    json!({"fs": {"readTextFile": true, "writeTextFile": true}})
}

fn session_id(result: &JsonValue) -> String {
    result["sessionId"].as_str().unwrap().to_owned()
}

fn prompt(session: &str, text: &str) -> JsonValue {
    json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]})
}

fn content(value: JsonValue) -> AssistantContent {
    serde_json::from_value(value).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_advertises_capabilities() {
    let fixture = Fixture::new(Vec::new());
    let client = fixture.connect(ClientPolicy::default());
    let result = client.initialize().await;
    assert_eq!(result["protocolVersion"], json!(1));
    assert_eq!(result["agentInfo"]["name"], json!("oino"));
    let caps = &result["agentCapabilities"];
    assert_eq!(caps["loadSession"], json!(true));
    assert_eq!(
        caps["promptCapabilities"],
        json!({"image": true, "audio": false, "embeddedContext": true})
    );
    assert_eq!(
        caps["mcpCapabilities"],
        json!({"http": false, "sse": false})
    );
    for capability in ["list", "resume", "close", "delete", "fork"] {
        assert_eq!(
            caps["sessionCapabilities"][capability],
            json!({}),
            "{capability}"
        );
    }
    assert_eq!(
        caps["_meta"]["oino"]["extensions"],
        json!(["_oino/extension_ui", "_oino/rpc"])
    );
    assert_eq!(result["authMethods"], json!([]));
    assert_eq!(
        client.ok("authenticate", json!({"methodId": "none"})).await,
        json!({})
    );
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_streams_thoughts_text_usage_and_commands() {
    let fixture = Fixture::new(vec![FauxResponse::Message {
        content: vec![
            content(json!({"type": "thinking", "thinking": "Let me think"})),
            content(json!({"type": "text", "text": "Hello from Oino"})),
        ],
        stop_reason: StopReason::Stop,
        usage: Usage::default(),
        error_message: None,
    }]);
    let client = fixture.connect(ClientPolicy::default());
    client.initialize().await;
    let created = client.new_session(&fixture.cwd()).await;
    let id = session_id(&created);
    let options = created["configOptions"].as_array().unwrap();
    let ids: Vec<_> = options.iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "model",
            "thinking",
            "steering_mode",
            "follow_up_mode",
            "auto_compaction",
            "auto_retry"
        ]
    );
    assert_eq!(options[0]["currentValue"], json!("faux/faux-1"));
    assert_eq!(options[0]["category"], json!("model"));
    assert_eq!(options[4]["type"], json!("boolean"));

    client
        .wait_for("available commands", |m| {
            m.iter().any(|m| {
                m["params"]["update"]["sessionUpdate"] == json!("available_commands_update")
            })
        })
        .await;
    let commands = &client.updates_of("available_commands_update")[0]["availableCommands"];
    let names: Vec<_> = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"compact") && names.contains(&"ask"),
        "{names:?}"
    );

    let result = client.ok("session/prompt", prompt(&id, "Hi")).await;
    assert_eq!(result, json!({"stopReason": "end_turn"}));
    assert_eq!(client.agent_text(), "Hello from Oino");
    let thoughts = client.updates_of("agent_thought_chunk");
    assert_eq!(
        thoughts[0]["content"],
        json!({"type": "text", "text": "Let me think"})
    );
    // Every update carries the session id.
    assert!(
        client
            .messages()
            .iter()
            .filter(|m| m["method"] == json!("session/update"))
            .all(|m| m["params"]["sessionId"] == json!(id))
    );
    let request = &fixture.provider.requests()[0];
    assert!(request.to_string().contains("Hi"), "{request}");
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn length_stop_is_max_tokens_and_errors_are_notices() {
    let fixture = Fixture::new(vec![
        FauxResponse::Message {
            content: vec![content(json!({"type": "text", "text": "cut"}))],
            stop_reason: StopReason::Length,
            // At the output limit: not a recoverable length stop (no automatic continuation).
            usage: Usage {
                output: 16_384,
                ..Usage::default()
            },
            error_message: None,
        },
        FauxResponse::Error("boom".into()),
    ]);
    let client = fixture.connect(ClientPolicy::default());
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    let result = client.ok("session/prompt", prompt(&id, "one")).await;
    assert_eq!(result["stopReason"], json!("max_tokens"));
    client
        .ok(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "auto_retry", "type": "boolean", "value": false}),
        )
        .await;
    let result = client.ok("session/prompt", prompt(&id, "two")).await;
    assert_eq!(result["stopReason"], json!("end_turn"));
    let notices = client.updates_of("notice");
    assert!(
        notices
            .iter()
            .any(|n| n["severity"] == json!("error") && n["title"] == json!("boom")),
        "{notices:#?}"
    );
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_calls_go_through_client_fs_with_diffs() {
    let fixture = Fixture::new(vec![
        tool_call(
            "call-1",
            "write",
            json!({"path": "notes.txt", "content": "new text\n"}),
        ),
        tool_call("call-2", "read", json!({"path": "notes.txt"})),
        FauxResponse::Text("done".into()),
    ]);
    std::fs::write(fixture.cwd().join("notes.txt"), "old text\n").unwrap();
    let client = fixture.connect(ClientPolicy {
        caps: fs_caps(),
        ..ClientPolicy::default()
    });
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    let result = client.ok("session/prompt", prompt(&id, "edit it")).await;
    assert_eq!(result["stopReason"], json!("end_turn"));
    let path = fixture.cwd().join("notes.txt");

    // The file went through the client.
    let writes = client.requests("fs/write_text_file");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0]["params"]["path"], json!(path));
    assert_eq!(writes[0]["params"]["content"], json!("new text\n"));
    assert_eq!(client.requests("fs/read_text_file").len(), 1);

    let calls = client.updates_of("tool_call");
    assert_eq!(calls[0]["toolCallId"], json!("call-1"));
    assert_eq!(calls[0]["kind"], json!("edit"));
    // `pending` is the default status and therefore omitted on the wire.
    assert!(
        calls[0].get("status").is_none_or(|s| s == "pending"),
        "{calls:#?}"
    );
    assert_eq!(calls[1]["kind"], json!("read"));

    let updates: Vec<_> = client
        .updates_of("tool_call_update")
        .into_iter()
        .filter(|u| u["toolCallId"] == json!("call-1"))
        .collect();
    assert!(updates.iter().any(
        |u| u["title"] == json!("Write notes.txt") && u["locations"] == json!([{"path": path}])
    ));
    assert!(updates.iter().any(|u| u["status"] == json!("in_progress")));
    let done = updates.last().unwrap();
    assert_eq!(done["status"], json!("completed"));
    let diff = done["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == json!("diff"))
        .unwrap();
    assert_eq!(diff["path"], json!(path));
    assert_eq!(diff["oldText"], json!("old text\n"));
    assert_eq!(diff["newText"], json!("new text\n"));
    assert!(done["rawOutput"].is_object());

    let read_done = client
        .updates_of("tool_call_update")
        .into_iter()
        .rfind(|u| u["toolCallId"] == json!("call-2"))
        .unwrap();
    assert!(read_done["content"].to_string().contains("new text"));
    assert!(
        client
            .updates_of("usage_update")
            .iter()
            .all(|u| u["size"] == json!(128_000))
    );
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_dialogs_become_permission_requests() {
    let fixture = Fixture::new(vec![
        tool_call("ls-1", "ls", json!({"path": "."})),
        tool_call("ls-2", "ls", json!({"path": "."})),
        FauxResponse::Text("ok".into()),
    ]);
    std::fs::write(fixture.cwd().join("visible.txt"), "x").unwrap();
    let client = fixture.connect(ClientPolicy {
        permissions: vec!["deny".into(), "allow".into()],
        ..ClientPolicy::default()
    });
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    client.ok("session/prompt", prompt(&id, "list")).await;
    let requests = client.requests("session/request_permission");
    assert_eq!(requests.len(), 2);
    let params = &requests[0]["params"];
    assert_eq!(params["sessionId"], json!(id));
    assert_eq!(params["toolCall"]["toolCallId"], json!("ls-1"));
    assert_eq!(params["toolCall"]["title"], json!("List a directory"));
    assert_eq!(params["toolCall"]["kind"], json!("search"));
    assert_eq!(
        params["options"],
        json!([
            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
            {"optionId": "deny", "name": "Deny", "kind": "reject_once"},
        ])
    );
    let last = |call: &str| {
        client
            .updates_of("tool_call_update")
            .into_iter()
            .rfind(|u| u["toolCallId"] == json!(call))
            .unwrap()
    };
    let denied = last("ls-1");
    assert_eq!(denied["status"], json!("failed"));
    assert!(denied["content"].to_string().contains("Denied by policy"));
    let allowed = last("ls-2");
    assert_eq!(allowed["status"], json!("completed"));
    assert!(allowed["content"].to_string().contains("visible.txt"));
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bash_runs_in_client_terminal() {
    let fixture = Fixture::new(vec![
        tool_call("sh-1", "bash", json!({"command": "echo from-terminal"})),
        FauxResponse::Text("ran".into()),
    ]);
    let client = fixture.connect(ClientPolicy {
        caps: json!({"terminal": true}),
        ..ClientPolicy::default()
    });
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    client.ok("session/prompt", prompt(&id, "run")).await;
    let created = client.requests("terminal/create");
    assert_eq!(created.len(), 1);
    let params = &created[0]["params"];
    assert!(
        params["args"].to_string().contains("echo from-terminal"),
        "{params}"
    );
    assert_eq!(params["cwd"], json!(fixture.cwd()));
    assert_eq!(client.requests("terminal/release").len(), 1);
    let updates: Vec<_> = client
        .updates_of("tool_call_update")
        .into_iter()
        .filter(|u| u["toolCallId"] == json!("sh-1"))
        .collect();
    let terminal = json!([{"type": "terminal", "terminalId": "term-1"}]);
    assert!(
        updates.iter().any(|u| u["content"] == terminal),
        "{updates:#?}"
    );
    let done = updates.last().unwrap();
    assert_eq!(done["status"], json!("completed"));
    assert_eq!(done["content"], terminal);
    assert!(done["rawOutput"].to_string().contains("from-terminal"));
    let calls = client.updates_of("tool_call");
    assert_eq!(calls[0]["kind"], json!("execute"));
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_aborts_the_turn() {
    let fixture = Fixture::new(vec![tool_call(
        "slow",
        "bash",
        json!({"command": "sleep 30"}),
    )]);
    let client = fixture.connect(ClientPolicy::default());
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    let started = std::time::Instant::now();
    let (result, ()) = tokio::join!(client.ok("session/prompt", prompt(&id, "wait")), async {
        client
            .wait_for("bash running", |m| {
                m.iter().any(|m| {
                    m["params"]["update"]["toolCallId"] == json!("slow")
                        && m["params"]["update"]["status"] == json!("in_progress")
                })
            })
            .await;
        client
            .notify("session/cancel", json!({"sessionId": id}))
            .await;
    });
    assert_eq!(result, json!({"stopReason": "cancelled"}));
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_replays_history_and_list_finds_it() {
    let fixture = Fixture::new(vec![
        tool_call("r-1", "read", json!({"path": "a.txt"})),
        FauxResponse::Text("The file says hi".into()),
    ]);
    std::fs::write(fixture.cwd().join("a.txt"), "hi").unwrap();
    let first = fixture.connect(ClientPolicy::default());
    first.initialize().await;
    let id = session_id(&first.new_session(&fixture.cwd()).await);
    first
        .ok("session/prompt", prompt(&id, "What is in a.txt?"))
        .await;
    first.close().await;

    let second = fixture.connect(ClientPolicy::default());
    second.initialize().await;
    let listed = second
        .ok("session/list", json!({"cwd": fixture.cwd()}))
        .await;
    let sessions = listed["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["sessionId"], json!(id));
    assert_eq!(sessions[0]["title"], json!("What is in a.txt?"));
    assert!(sessions[0]["updatedAt"].is_string());

    let loaded = second
        .ok(
            "session/load",
            json!({"sessionId": id, "cwd": fixture.cwd(), "mcpServers": []}),
        )
        .await;
    assert!(loaded["configOptions"].is_array());
    let replay: Vec<_> = second
        .updates()
        .into_iter()
        .filter(|u| u["sessionUpdate"] != json!("available_commands_update"))
        .collect();
    assert_eq!(replay[0]["sessionUpdate"], json!("user_message_chunk"));
    assert_eq!(replay[0]["content"]["text"], json!("What is in a.txt?"));
    assert_eq!(replay[1]["sessionUpdate"], json!("tool_call"));
    assert_eq!(replay[1]["title"], json!("Read a.txt"));
    assert_eq!(replay[2]["sessionUpdate"], json!("tool_call_update"));
    assert_eq!(replay[2]["status"], json!("completed"));
    assert_eq!(replay[3]["sessionUpdate"], json!("agent_message_chunk"));
    assert_eq!(replay[3]["content"]["text"], json!("The file says hi"));

    // Resume of a live session does not replay again.
    let before = second.updates().len();
    second
        .ok(
            "session/resume",
            json!({"sessionId": id, "cwd": fixture.cwd()}),
        )
        .await;
    assert!(
        second.updates()[before..]
            .iter()
            .all(|u| u["sessionUpdate"] == json!("available_commands_update"))
    );

    // Unknown sessions are errors.
    let missing = second
        .call(
            "session/load",
            json!({"sessionId": "nope", "cwd": fixture.cwd(), "mcpServers": []}),
        )
        .await;
    assert!(
        missing["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nope")
    );
    second.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_options_and_modes() {
    let fixture = Fixture::new(Vec::new());
    let client = fixture.connect(ClientPolicy::default());
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    let result = client
        .ok(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "thinking", "value": "high"}),
        )
        .await;
    let thinking = result["configOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == json!("thinking"))
        .unwrap()
        .clone();
    assert_eq!(thinking["currentValue"], json!("high"));
    client
        .wait_for("config update", |m| {
            m.iter()
                .any(|m| m["params"]["update"]["sessionUpdate"] == json!("config_option_update"))
        })
        .await;

    let result = client
        .ok(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "steering_mode", "value": "all"}),
        )
        .await;
    assert!(result.to_string().contains(r#""currentValue":"all""#));
    client
        .ok(
            "session/set_mode",
            json!({"sessionId": id, "modeId": "low"}),
        )
        .await;
    let state = client
        .ok(
            "_oino/rpc",
            json!({"sessionId": id, "command": {"type": "get_state"}}),
        )
        .await;
    assert_eq!(state["thinkingLevel"], json!("low"));
    assert_eq!(state["steeringMode"], json!("all"));

    let bad = client
        .call(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "thinking", "value": "galaxy-brain"}),
        )
        .await;
    assert_eq!(bad["error"]["code"], json!(-32602));
    let unknown = client
        .call(
            "session/set_config_option",
            json!({"sessionId": "missing", "configId": "thinking", "value": "low"}),
        )
        .await;
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Session not found")
    );
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slash_commands_ext_methods_and_elicitation() {
    let fixture = Fixture::new(Vec::new());
    let client = fixture.connect(ClientPolicy {
        caps: json!({"elicitation": {"form": {}}}),
        elicitations: vec![json!("b")],
        ..ClientPolicy::default()
    });
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);

    let result = client
        .ok("session/prompt", prompt(&id, "/name Refactor"))
        .await;
    assert_eq!(result["stopReason"], json!("end_turn"));
    assert!(
        client
            .agent_text()
            .contains("Session name set to Refactor.")
    );
    client
        .wait_for("title", |m| {
            m.iter()
                .any(|m| m["params"]["update"]["title"] == json!("Refactor"))
        })
        .await;
    client.ok("session/prompt", prompt(&id, "/session")).await;
    assert!(client.agent_text().contains(&format!("Session: {id}")));

    // An extension command raising a select dialog → elicitation/create.
    let result = client.ok("session/prompt", prompt(&id, "/ask")).await;
    assert_eq!(result["stopReason"], json!("end_turn"));
    client
        .wait_for("notice", |m| {
            m.iter()
                .any(|m| m["params"]["update"]["title"] == json!("picked {\"value\":\"b\"}"))
        })
        .await;
    let elicitation = &client.requests("elicitation/create")[0]["params"];
    assert_eq!(elicitation["mode"], json!("form"));
    assert_eq!(elicitation["sessionId"], json!(id));
    assert_eq!(
        elicitation["requestedSchema"]["properties"]["value"]["enum"],
        json!(["a", "b"])
    );

    // `_oino/<command>` with the session id.
    let usage = client
        .ok("_oino/get_context_usage", json!({"sessionId": id}))
        .await;
    assert_eq!(usage["contextWindow"], json!(128_000));
    let bad = client
        .call(
            "_oino/rpc",
            json!({"sessionId": id, "command": {"type": "nope"}}),
        )
        .await;
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown command: nope")
    );
    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_close_and_delete() {
    let fixture = Fixture::new(vec![FauxResponse::Text("first".into())]);
    let client = fixture.connect(ClientPolicy::default());
    client.initialize().await;
    let id = session_id(&client.new_session(&fixture.cwd()).await);
    client
        .ok("session/prompt", prompt(&id, "remember me"))
        .await;

    let forked = client
        .ok(
            "session/fork",
            json!({"sessionId": id, "cwd": fixture.cwd()}),
        )
        .await;
    let fork_id = session_id(&forked);
    assert_ne!(fork_id, id);
    let messages = client
        .ok(
            "_oino/rpc",
            json!({"sessionId": fork_id, "command": {"type": "get_messages"}}),
        )
        .await;
    assert!(messages.to_string().contains("remember me"));

    let listed = client.ok("session/list", json!({})).await;
    assert_eq!(listed["sessions"].as_array().unwrap().len(), 2);

    client
        .ok("session/close", json!({"sessionId": fork_id}))
        .await;
    let gone = client.call("session/prompt", prompt(&fork_id, "hi")).await;
    assert!(gone["error"].is_object());

    client.ok("session/delete", json!({"sessionId": id})).await;
    let listed = client
        .ok("session/list", json!({"cwd": fixture.cwd()}))
        .await;
    let remaining: Vec<_> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sessionId"].clone())
        .collect();
    assert_eq!(remaining, vec![json!(fork_id)]);
    let trash = fixture.dir.path().join("agent").join("trash");
    assert_eq!(std::fs::read_dir(trash).unwrap().count(), 1);
    client.close().await;
}

/// The official SDK client parses every response and update (strict typed decoding).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdk_client_interop() {
    use agent_client_protocol::schema::ProtocolVersion;
    use agent_client_protocol::schema::v1::{
        ContentBlock, InitializeRequest, ListSessionsRequest, NewSessionRequest, PromptRequest,
        SessionNotification, SessionUpdate, StopReason as AcpStop, TextContent,
    };
    use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
    use std::sync::{Arc, Mutex};
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let fixture = Fixture::new(vec![
        tool_call("w", "write", json!({"path": "x.txt", "content": "x"})),
        FauxResponse::Text("typed hello".into()),
    ]);
    let (client_io, agent_io) = tokio::io::duplex(1 << 20);
    let (agent_read, agent_write) = tokio::io::split(agent_io);
    let agent = oino_acp::AcpAgent::new(
        fixture.factory.clone(),
        oino_acp::AgentInfo::default(),
        fixture.cwd(),
    );
    let server = tokio::spawn(async move {
        let _ = agent
            .serve(ByteStreams::new(
                agent_write.compat_write(),
                agent_read.compat(),
            ))
            .await;
    });
    let (client_read, client_write) = tokio::io::split(client_io);
    let updates: Arc<Mutex<Vec<SessionUpdate>>> = Arc::default();
    let seen = Arc::clone(&updates);
    let cwd = fixture.cwd();
    Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                seen.lock().unwrap().push(notification.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            ByteStreams::new(client_write.compat_write(), client_read.compat()),
            async move |cx: ConnectionTo<Agent>| {
                let init = cx
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                assert!(init.agent_capabilities.load_session);
                let session = cx
                    .send_request(NewSessionRequest::new(cwd.clone()))
                    .block_task()
                    .await?;
                assert_eq!(session.config_options.as_ref().map(Vec::len), Some(6));
                let response = cx
                    .send_request(PromptRequest::new(
                        session.session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new("hi"))],
                    ))
                    .block_task()
                    .await?;
                assert_eq!(response.stop_reason, AcpStop::EndTurn);
                let listed = cx
                    .send_request(ListSessionsRequest::new())
                    .block_task()
                    .await?;
                assert_eq!(listed.sessions.len(), 1);
                Ok(())
            },
        )
        .await
        .unwrap();
    let updates = updates.lock().unwrap();
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, SessionUpdate::AgentMessageChunk(_)))
    );
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, SessionUpdate::ToolCall(_)))
    );
    assert!(
        updates
            .iter()
            .any(|u| matches!(u, SessionUpdate::AvailableCommandsUpdate(_)))
    );
    server.abort();
}
