//! Echo fixture: registration, tools, commands, events, and every host import (allowed +
//! denied) through `probe`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use oino_extension_api::{
    CallKind, CancellationToken, EventKind, ExtensionRuntime, InitInput, RawJson, ToolInvocation,
};
use oino_wasm_host::testing::Harness;
use serde_json::{Value, json};

async fn probe(
    h: &Harness,
    ext: &dyn oino_extension_api::ExtensionInstance,
    name: &str,
    arg: Value,
) -> Value {
    let out = ext
        .run_command(
            "probe",
            &format!("{name} {arg}"),
            CancellationToken::new(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap();
    out.to_value()
}

#[tokio::test(flavor = "multi_thread")]
async fn init_tool_command_events() {
    let h = Harness::new().unwrap();
    let ext = h.load("echo", None).await.unwrap();
    let reg = ext
        .init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    assert_eq!(reg.tools[0].name, "echo");
    assert_eq!(reg.tools[0].label, "Echo");
    assert!(reg.tools[0].parameters.to_value()["properties"]["text"].is_object());
    assert!(reg.events.contains(&EventKind::ToolCall));
    assert_eq!(reg.events.iter().filter(|k| !k.is_oino()).count(), 41);
    assert_eq!(reg.commands.len(), 2);
    assert_eq!(reg.shortcuts[0].id, "echo.shortcut");
    assert_eq!(reg.flags[0].name, "echo-verbose");
    assert_eq!(reg.services, vec!["echo.ping"]);

    let inv = ToolInvocation {
        tool_call_id: "t1".into(),
        name: "echo".into(),
        args: RawJson::from(json!({"text": "hi"})),
        parent_tool_call_id: None,
    };
    let result = ext
        .run_tool(inv, CancellationToken::new(), h.cx("s1", CallKind::Tool))
        .await
        .unwrap()
        .to_value();
    assert_eq!(result["content"][0]["text"], "hi");
    assert_eq!(result["details"]["session"], "s1");
    assert_eq!(h.host.calls_named("tool_progress").len(), 1);

    let out = ext
        .run_command(
            "echo",
            "a b",
            CancellationToken::new(),
            h.cx("s1", CallKind::Command),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out.to_value()["args"], "a b");
    let items = ext
        .complete_command("echo", "x", h.cx("s1", CallKind::Command))
        .await;
    assert_eq!(
        items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(),
        ["x-one", "x-two"]
    );

    let r = ext
        .on_event(
            EventKind::ToolCall,
            RawJson::from(json!({"type": "tool_call", "toolName": "blocked"})),
            h.cx("s2", CallKind::Event),
        )
        .await
        .unwrap();
    assert_eq!(r.unwrap().to_value()["block"], true);
    let r = ext
        .on_event(
            EventKind::AgentEnd,
            RawJson::from(json!({"type": "agent_end", "messages": []})),
            h.cx("s2", CallKind::Event),
        )
        .await
        .unwrap();
    assert!(r.is_none());
    let entries = h.host.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[1]["data"],
        json!({"kind": "agent_end", "type": "agent_end"})
    );
    ext.run_shortcut("echo.shortcut", h.cx("s1", CallKind::Shortcut))
        .await
        .unwrap();
    assert_eq!(h.host.entries()[2]["customType"], "echo.shortcut");
}

#[tokio::test(flavor = "multi_thread")]
async fn imports_allowed() {
    let h = Harness::new().unwrap();
    let ext = h.load("echo", None).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();

    let info = probe(&h, &*ext, "host-info", Value::Null).await;
    assert_eq!(info["ok"]["api"], "1.0.0");
    assert_eq!(info["ok"]["ext"], "1.0.0");
    assert_eq!(info["ok"]["ws"].as_str().unwrap().len(), 16);
    assert_eq!(
        probe(&h, &*ext, "env", json!({"name": "HOME"})).await["ok"],
        json!(std::env::var("HOME").ok())
    );
    assert!(probe(&h, &*ext, "log", Value::Null).await["ok"].is_null());
    let log =
        std::fs::read_to_string(h.agent_dir.path().join("logs/ext/oino.fixture.echo.log")).unwrap();
    assert!(log.contains("probe log line"));

    let kv = probe(&h, &*ext, "kv", Value::Null).await;
    assert_eq!(kv["ok"]["global"], "v1");
    assert_eq!(kv["ok"]["session"], json!({"n": 1}));
    assert!(
        h.agent_dir
            .path()
            .join("state/oino.fixture.echo/session/s1/kv.json")
            .exists()
    );

    let fs = probe(&h, &*ext, "fs", Value::Null).await;
    assert_eq!(fs["ok"]["text"], "hello");
    assert!(
        fs["ok"]["preopens"]
            .as_array()
            .unwrap()
            .contains(&json!("project"))
    );
    assert!(
        probe(&h, &*ext, "fs-write-project", Value::Null).await["err"]
            .as_str()
            .unwrap()
            .contains("denied")
    );
    assert_eq!(
        probe(&h, &*ext, "fs-lock", Value::Null).await["ok"],
        "locked"
    );

    let exec = probe(
        &h,
        &*ext,
        "exec",
        json!({"script": "echo out; echo err >&2; exit 4"}),
    )
    .await;
    assert_eq!(
        exec["ok"],
        json!({"stdout": "out\n", "stderr": "err\n", "code": 4, "killed": false})
    );
    let spawn = probe(&h, &*ext, "spawn", json!({"input": "piped"})).await;
    assert_eq!(
        spawn["ok"],
        json!({"stdout": "piped", "code": 0, "pid": true})
    );

    assert_eq!(probe(&h, &*ext, "sleep", Value::Null).await["ok"], true);
    assert_eq!(probe(&h, &*ext, "secrets", Value::Null).await["ok"], "abc");
    let secrets = std::fs::read_to_string(h.agent_dir.path().join("secrets.json")).unwrap();
    assert!(secrets.contains("oino.fixture.echo/token"));

    assert_eq!(
        probe(
            &h,
            &*ext,
            "bus-publish",
            json!({"topic": "t:x", "data": {"a": 1}})
        )
        .await["ok"],
        json!({"a": 1})
    );
    assert_eq!(
        probe(&h, &*ext, "session", Value::Null).await["ok"]["id"],
        "s1"
    );
    let id = probe(&h, &*ext, "append", json!({"type": "x", "data": {"b": 2}})).await;
    assert!(id["ok"].is_string());
    assert!(probe(&h, &*ext, "send-message", json!({"text": "m"})).await["ok"].is_null());
    assert!(probe(&h, &*ext, "set-name", json!({"name": "n"})).await["ok"].is_null());
    assert_eq!(
        probe(&h, &*ext, "control", Value::Null).await["ok"],
        json!({"idle": true, "thinking": "off"})
    );
    assert!(probe(&h, &*ext, "ui-notify", Value::Null).await["ok"].is_null());
    assert!(probe(&h, &*ext, "os-notify", Value::Null).await["ok"].is_null());
    assert_eq!(
        probe(&h, &*ext, "listener", Value::Null).await["ok"]["port"],
        true
    );
    assert!(probe(&h, &*ext, "register-tool", json!({"name": "dyn"})).await["ok"].is_null());
    assert!(
        h.host
            .calls_named("register_tool")
            .iter()
            .any(|c| c.detail["name"] == "dyn")
    );
    // Command-context-only op from a command: forwarded (recorder answers null → false).
    assert!(
        probe(&h, &*ext, "new-session", Value::Null)
            .await
            .get("ok")
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn imports_denied_without_grants() {
    let h = Harness::new().unwrap();
    let ext = h.load("echo", Some(json!({}))).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    let denied = |v: Value, perm: &str| {
        let err = v["err"].as_str().unwrap_or_default().to_owned();
        assert!(
            err.contains("permission denied") && err.contains(perm),
            "{perm}: {v}"
        );
    };
    denied(
        probe(&h, &*ext, "env", json!({"name": "HOME"})).await,
        "env:HOME",
    );
    denied(
        probe(&h, &*ext, "exec", json!({"script": "true"})).await,
        "process.allow:sh",
    );
    denied(
        probe(&h, &*ext, "spawn", json!({"input": ""})).await,
        "process.allow:cat",
    );
    denied(
        probe(&h, &*ext, "http", json!({"url": "http://127.0.0.1:9/"})).await,
        "network.hosts:127.0.0.1",
    );
    denied(
        probe(&h, &*ext, "listener", Value::Null).await,
        "network.listenLoopback",
    );
    denied(probe(&h, &*ext, "secrets", Value::Null).await, "secrets");
    denied(
        probe(&h, &*ext, "set-name", json!({"name": "n"})).await,
        "sessionControl",
    );
    denied(
        probe(&h, &*ext, "set-model", Value::Null).await,
        "sessionControl",
    );
    denied(
        probe(&h, &*ext, "list-sessions", Value::Null).await,
        "sessions.readAll",
    );
    denied(
        probe(&h, &*ext, "complete", Value::Null).await,
        "models.complete",
    );
    denied(
        probe(&h, &*ext, "credential", json!({"provider": "openai"})).await,
        "credentials:openai",
    );
    denied(probe(&h, &*ext, "agent", Value::Null).await, "agents.spawn");
    denied(
        probe(&h, &*ext, "task", json!({"command": "true"})).await,
        "process.spawn",
    );
    denied(probe(&h, &*ext, "os-notify", Value::Null).await, "osNotify");
    denied(
        probe(&h, &*ext, "service", json!({"name": "x", "payload": null})).await,
        "services.call",
    );
    // Always-on capabilities still work.
    assert_eq!(
        probe(&h, &*ext, "kv", Value::Null).await["ok"]["global"],
        "v1"
    );
    assert!(
        !probe(&h, &*ext, "fs", Value::Null).await["ok"]["preopens"]
            .as_array()
            .unwrap()
            .contains(&json!("project"))
    );
    // ui defaults to granted; explicit false denies.
    let h2 = Harness::new().unwrap();
    let ext2 = h2.load("echo", Some(json!({"ui": false}))).await.unwrap();
    ext2.init(InitInput::default(), h2.cx("s1", CallKind::Init))
        .await
        .unwrap();
    denied(probe(&h2, &*ext2, "ui-notify", Value::Null).await, "ui");
    // A non-command context cannot use command-context-only operations.
    let r = ext
        .on_event(
            EventKind::Input,
            RawJson::from(json!({"type": "input", "text": "x"})),
            h.cx("s1", CallKind::Event),
        )
        .await;
    assert!(r.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn http_allowlist_and_timers() {
    let h = Harness::new().unwrap();
    let ext = h.load("echo", None).await.unwrap();
    ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
        .await
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
        }
    });
    let r = probe(
        &h,
        &*ext,
        "http",
        json!({"url": format!("http://127.0.0.1:{port}/x")}),
    )
    .await;
    assert_eq!(r["ok"], json!({"status": 200, "body": "ok"}));
    let r = probe(&h, &*ext, "http", json!({"url": "http://localhost:9/"})).await;
    assert!(
        r["err"]
            .as_str()
            .unwrap()
            .contains("network.hosts:localhost")
    );

    probe(&h, &*ext, "timer", json!({"id": "tick", "ms": 10})).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let delivered = h.host.calls_named("deliver_event");
    // The local hub delivers timer events straight to the instance; the instance records them.
    assert_eq!(delivered.len(), 0);
    assert!(
        h.host
            .entries()
            .iter()
            .any(|e| e["data"]["kind"] == "oino_timer"),
        "{:?}",
        h.host.entries()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dist_packages_validate_and_load_fast() {
    let dist = oino_wasm_host::testing::repo_root().join("extensions/dist");
    let (packages, problems) = oino_wasm_host::packages::discover(&[dist], true);
    assert!(problems.is_empty(), "{problems:?}");
    if packages.is_empty() {
        return; // fresh clone without `cargo xtask wasm`
    }
    let h = Harness::new().unwrap();
    // Warm the compile cache, then time a cold start (load + init) of every component.
    for p in &packages {
        let _ = h.runtime.load(p.clone(), h.host.clone()).await.unwrap();
    }
    let started = std::time::Instant::now();
    for p in &packages {
        let ext = h.runtime.load(p.clone(), h.host.clone()).await.unwrap();
        ext.init(InitInput::default(), h.cx("s1", CallKind::Init))
            .await
            .unwrap();
    }
    let per = started.elapsed() / u32::try_from(packages.len()).unwrap();
    assert!(per < std::time::Duration::from_millis(500), "{per:?}");
}
