//! ui-probe, async-probe and provider-faux fixtures: UI round trips, concurrency, deadlines,
//! cancellation, services between instances, bus, tasks, traps and provider streaming.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use oino_extension_api::{
    CallKind, CancellationToken, EventKind, ExtensionInstance, InitInput, RawJson, ToolInvocation,
};
use oino_wasm_host::testing::Harness;
use serde_json::{Value, json};

fn inv(name: &str, id: &str) -> ToolInvocation {
    ToolInvocation {
        tool_call_id: id.into(),
        name: name.into(),
        args: RawJson::from(json!({})),
        parent_tool_call_id: None,
    }
}

async fn loaded(h: &Harness, name: &str) -> Arc<dyn ExtensionInstance> {
    let ext = h.load(name, None).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    ext
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_probe_round_trips_every_request() {
    let h = Harness::new().unwrap();
    let ext = loaded(&h, "ui-probe").await;
    for i in 0..8 {
        h.host
            .queue_dialog_answer(json!({"value": format!("a{i}")}));
    }
    h.host.queue_dialog_answer(json!({"confirmed": true}));
    let out = ext
        .run_command(
            "ui-probe",
            "",
            CancellationToken::new(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert_eq!(out["answers"].as_array().unwrap().len(), 8);
    assert_eq!(out["answers"][7]["value"], "a7");
    assert_eq!(out["editor"], "draft!");
    assert_eq!(out["confirmed"], true);
    assert_eq!(out["panel"], "panel-1");
    let dialogs: Vec<String> = h
        .host
        .calls_named("ui_dialog")
        .iter()
        .map(|c| c.detail["method"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        &dialogs[..8],
        [
            "select",
            "confirm",
            "input",
            "editor",
            "multiSelect",
            "form",
            "approval",
            "custom"
        ]
    );
    assert_eq!(h.host.calls_named("ui_fire").len(), 9);
    assert_eq!(h.host.calls_named("ui_panel").len(), 3);
    assert_eq!(h.host.calls_named("ui_device").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_waits_on_dialog_while_events_run() {
    let h = Harness::new().unwrap();
    let ext = loaded(&h, "async-probe").await;
    let e2 = Arc::clone(&ext);
    let cx = h.cx("s1", CallKind::Tool);
    let tool = tokio::spawn(async move {
        e2.run_tool(inv("wait_dialog", "t1"), CancellationToken::new(), cx)
            .await
    });
    for _ in 0..100 {
        if h.host.pending_dialogs() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(h.host.pending_dialogs(), 1);
    for _ in 0..3 {
        ext.on_event(
            EventKind::TurnStart,
            RawJson::from(json!({"type": "turn_start"})),
            h.cx("s1", CallKind::Event),
        )
        .await
        .unwrap();
    }
    assert!(h.host.answer_pending_dialog(json!({"confirmed": true})));
    let result = tool.await.unwrap().unwrap().to_value();
    assert_eq!(result["details"]["eventsDuring"], 3);
    assert_eq!(result["details"]["answer"]["confirmed"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn deadlines_cancellation_and_trap_restart() {
    let h = Harness::new().unwrap();
    let ext = loaded(&h, "async-probe").await;
    let started = std::time::Instant::now();
    let err = ext
        .on_event(
            EventKind::AgentStart,
            RawJson::from(json!({"type": "agent_start"})),
            h.cx("s1", CallKind::Event),
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("timed out"), "{err:?}");
    assert!(started.elapsed() < Duration::from_millis(1500));

    let token = CancellationToken::new();
    let e2 = Arc::clone(&ext);
    let (t2, cx) = (token.clone(), h.cx("s1", CallKind::Tool));
    let call = tokio::spawn(async move { e2.run_tool(inv("wait_cancel", "t2"), t2, cx).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    token.cancel();
    assert_eq!(
        call.await.unwrap().unwrap().to_value()["content"][0]["text"],
        "cancelled"
    );

    ext.on_event(
        EventKind::TurnStart,
        RawJson::from(json!({"type": "turn_start"})),
        h.cx("s1", CallKind::Event),
    )
    .await
    .unwrap();
    let err = ext
        .run_tool(
            inv("crash", "t3"),
            CancellationToken::new(),
            h.cx("s1", CallKind::Tool),
        )
        .await
        .unwrap_err();
    assert!(err.trap, "{err:?}");
    // Restarted lazily with init replayed: state reset, tools still served.
    let count = ext
        .run_tool(
            inv("count", "t4"),
            CancellationToken::new(),
            h.cx("s1", CallKind::Tool),
        )
        .await
        .unwrap();
    assert_eq!(count.to_value()["content"][0]["text"], "0");
    for i in 0..3 {
        let _ = ext
            .run_tool(
                inv("crash", &format!("c{i}")),
                CancellationToken::new(),
                h.cx("s1", CallKind::Tool),
            )
            .await;
    }
    let err = ext
        .run_tool(
            inv("count", "t5"),
            CancellationToken::new(),
            h.cx("s1", CallKind::Tool),
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("disabled"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn services_bus_and_tasks_between_instances() {
    let h = Harness::new().unwrap();
    let a = loaded(&h, "async-probe").await;
    let echo = loaded(&h, "echo").await;
    let cmd = |ext: &Arc<dyn ExtensionInstance>, args: &str| {
        let ext = Arc::clone(ext);
        let (args, cx) = (args.to_owned(), h.cx("s1", CallKind::Command));
        async move {
            ext.run_command(
                args.split_once(' ').map_or(args.as_str(), |p| p.0),
                args.split_once(' ').map_or("", |p| p.1),
                CancellationToken::new(),
                cx,
            )
            .await
        }
    };
    // async-probe → echo's service, and echo → async-probe's `add`.
    let out = cmd(&a, "call oino.fixture.echo echo.ping")
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert_eq!(out["pong"], json!({"a": 2, "b": 3}));
    assert_eq!(out["caller"], "oino.fixture.async-probe");
    let out = cmd(&a, "call  add").await.unwrap().unwrap().to_value();
    assert_eq!(out, json!(5));
    let probe = format!(
        "probe service {}",
        json!({"name": "add", "payload": {"a": 40, "b": 2}})
    );
    let out = cmd(&echo, &probe).await.unwrap().unwrap().to_value();
    assert_eq!(out["ok"], json!(42));
    let missing = format!("probe service {}", json!({"name": "nope", "payload": null}));
    assert!(
        cmd(&echo, &missing).await.unwrap().unwrap().to_value()["err"]
            .as_str()
            .unwrap()
            .contains("not found")
    );

    cmd(&a, "bus sub news:*").await.unwrap();
    let publish = format!(
        "probe bus-publish {}",
        json!({"topic": "news:x", "data": {"n": 1}})
    );
    cmd(&echo, &publish).await.unwrap();
    let mut seen = false;
    for _ in 0..100 {
        if h.host
            .entries()
            .iter()
            .any(|e| e["customType"] == "async.bus" && e["data"]["topic"] == "news:x")
        {
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(seen, "{:?}", h.host.entries());

    let id = cmd(&a, "task echo done; exit 3")
        .await
        .unwrap()
        .unwrap()
        .to_value();
    assert!(id.is_string());
    let mut notified = false;
    for _ in 0..300 {
        let task_event = h
            .host
            .entries()
            .iter()
            .any(|e| e["customType"] == "async.task" && e["data"]["exitCode"] == 3);
        let message = h
            .host
            .calls_named("session_write")
            .iter()
            .any(|c| c.detail["sendMessage"]["customType"] == "background-task-notification");
        if task_event && message {
            notified = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(notified, "{:?}", h.host.calls());
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_faux_streams_script() {
    let h = Harness::new().unwrap();
    let ext = loaded(&h, "provider-faux").await;
    assert!(
        h.host
            .calls_named("register_provider")
            .iter()
            .any(|c| c.detail["id"] == "faux")
    );
    let provider = ext.provider().unwrap();
    let models = provider.list_models("faux", None).await.unwrap();
    assert_eq!(models[0].id, "faux-1");
    let request = json!({"requestId": "r1", "model": {"id": "faux-1", "api": "faux", "provider": "faux"}, "context": {"messages": []},
        "options": {"metadata": {"fauxScript": [{"text": "a"}, {"toolCall": {"id": "c1", "name": "echo", "arguments": {"text": "x"}}}]}}});
    let events: Vec<Value> = provider
        .stream(RawJson::from(request))
        .await
        .unwrap()
        .map(|e| serde_json::to_value(e.unwrap()).unwrap())
        .collect()
        .await;
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(
        types,
        [
            "start",
            "text_start",
            "text_delta",
            "text_end",
            "toolcall_start",
            "toolcall_end",
            "done"
        ]
    );
    assert_eq!(events[6]["reason"], "toolUse");
    let err = provider
        .stream(RawJson::from(json!({"requestId": "r2", "model": {}, "options": {"metadata": {"fauxScript": [{"error": "boom"}]}}})))
        .await
        .unwrap()
        .map(|e| serde_json::to_value(e.unwrap()).unwrap())
        .collect::<Vec<_>>()
        .await;
    assert_eq!(err.last().unwrap()["type"], "error");
    // provider-host hooks reached HostServices::provider_hook (recorder passes through).
    let default = provider
        .stream(RawJson::from(json!({"requestId": "r3", "model": {}})))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(default.len(), 5);
}
