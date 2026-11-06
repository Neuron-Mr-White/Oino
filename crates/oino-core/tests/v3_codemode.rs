//! End-to-end sandbox/discovery acceptance using real built-ins and session hooks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use oino_agent::AgentToolResult;
use oino_ai::{AgentMessage, JsonObject, UserContent};
use oino_core::{
    builtin::{SessionToolContext, builtin_tool_entries},
    extensions::{
        api::{Registration, ToolSpec},
        host::HostOp,
    },
    session::{AgentSession, PromptOptions},
    testing::{FakeExtension, FauxProvider, FauxResponse, TestSession},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn args(v: Value) -> JsonObject {
    v.as_object().unwrap().clone()
}
fn spec(name: &str) -> ToolSpec {
    serde_json::from_value(json!({"name":name,"label":name,"description":format!("{name} tool"),"parameters":{"type":"object","properties":{},"additionalProperties":false}})).unwrap()
}
fn call(id: &str, name: &str, args: Value) -> FauxResponse {
    FauxResponse::ToolCalls(vec![(id.into(), name.into(), self::args(args))])
}
async fn run(session: &AgentSession) {
    session
        .prompt("go".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}
fn result(session: &AgentSession, id: &str) -> (String, bool) {
    session
        .messages()
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) if r.tool_call_id == id => Some((
                r.content
                    .iter()
                    .filter_map(|v| match v {
                        UserContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                r.is_error,
            )),
            _ => None,
        })
        .unwrap()
}
async fn build(
    responses: Vec<FauxResponse>,
    extension: Arc<FakeExtension>,
    settings: Value,
    dir: &std::path::Path,
) -> (AgentSession, Arc<FauxProvider>) {
    let provider = FauxProvider::new(responses);
    let settings = args(settings);
    let snapshot = oino_settings::Settings::from_layers(
        settings.clone(),
        Default::default(),
        Default::default(),
        None,
    );
    let context = Arc::new(SessionToolContext::default());
    let tools = builtin_tool_entries(dir, &dir.join("agent"), &snapshot, context.clone());
    let session = TestSession {
        tools,
        settings,
        cwd: dir.to_path_buf(),
        extensions: vec![extension],
        ..Default::default()
    }
    .build(provider.clone())
    .await;
    context.bind(&session);
    (session, provider)
}

#[tokio::test]
async fn search_and_deferred_reload() {
    let dir = tempfile::tempdir().unwrap();
    let ext = Arc::new(
        FakeExtension::new(
            "mcp",
            Registration {
                tools: (0..7).map(|i| spec(&format!("mcp_search_{i}"))).collect(),
                ..Default::default()
            },
        )
        .on_tool(|name, _| Ok(AgentToolResult::text(name))),
    );
    let (session, provider) = build(
        vec![
            call(
                "s1",
                "tool_search",
                json!({"query":"mcp search","max_results":null}),
            ),
            // Newly loaded tools must execute on the very next in-turn request.
            call("s2", "mcp_search_0", json!({})),
            FauxResponse::Text("done".into()),
            call(
                "s3",
                "tool_search",
                json!({"query":"select:mcp_search_6,mcp_search_5","max_results":1}),
            ),
            FauxResponse::Text("done".into()),
        ],
        ext,
        json!({}),
        dir.path(),
    )
    .await;
    let original = session.agent().with_state(|s| {
        s.tools
            .iter()
            .map(|t| t.meta().declaration())
            .collect::<Vec<_>>()
    });
    assert!(
        !session
            .active_tool_names()
            .iter()
            .any(|n| n.starts_with("mcp_"))
    );
    run(&session).await;
    assert_eq!(session.loaded_deferred_tool_names().len(), 5);
    assert_eq!(result(&session, "s2"), ("mcp_search_0".into(), false));
    let request_text = provider.requests()[0].to_string();
    assert!(request_text.contains("Deferred tools:"));
    assert!(request_text.contains("mcp_search_6"));
    let loaded = session.agent().with_state(|s| {
        s.tools
            .iter()
            .map(|t| t.meta().declaration())
            .collect::<Vec<_>>()
    });
    assert_eq!(&loaded[..original.len()], &original);
    assert!(
        session
            .search_deferred_tools("select:mcp_search_0,not_real", None)
            .is_err()
    );
    assert_eq!(session.agent().with_state(|s| s.tools.len()), loaded.len());
    run(&session).await;
    assert_eq!(session.loaded_deferred_tool_names().len(), 7);
    let count = session.active_tool_names().len();
    session
        .search_deferred_tools("select:mcp_search_6,mcp_search_6", Some(1))
        .unwrap();
    assert_eq!(session.active_tool_names().len(), count);
    session.reload().await.unwrap();
    assert_eq!(session.loaded_deferred_tool_names().len(), 7);
    session.rebuild_declarations_after_compaction();
    assert_eq!(session.loaded_deferred_tool_names().len(), 7);
    session
        .host_call(
            "mcp",
            session.runner().call_context(),
            HostOp::RegisterTool(spec("mcp_late")),
        )
        .await
        .unwrap();
    assert!(!session.active_tool_names().contains(&"mcp_late".into()));
    assert!(session.with_manager(|m| {
        m.branch(None)
            .iter()
            .any(|e| serde_json::to_string(e).unwrap().contains("mcp_late"))
    }));
    session
        .search_deferred_tools("select:mcp_late", None)
        .unwrap();
    session
        .host_call(
            "mcp",
            session.runner().call_context(),
            HostOp::UnregisterTool("mcp_late".into()),
        )
        .await
        .unwrap();
    assert!(
        session
            .search_deferred_tools("select:mcp_late", None)
            .is_err()
    );
    assert!(session.active_tool_names().contains(&"mcp_late".into()));
    session.rebuild_declarations_after_compaction();
    assert!(!session.active_tool_names().contains(&"mcp_late".into()));
}

#[tokio::test]
async fn sandbox_and_permission_gate() {
    let dir = tempfile::tempdir().unwrap();
    let effects = Arc::new(AtomicUsize::new(0));
    let hook_calls = Arc::new(Mutex::new(Vec::new()));
    let effects2 = effects.clone();
    let hook_calls2 = hook_calls.clone();
    let ext = Arc::new(
        FakeExtension::new(
            "mcp",
            Registration {
                tools: vec![
                    spec("mcp_ok"),
                    spec("mcp_blocked"),
                    spec("mcp__hyphen-name"),
                ],
                events: vec!["tool_call".into()],
                ..Default::default()
            },
        )
        .on_event(move |kind, value| {
            if kind == "tool_call" {
                hook_calls2.lock().unwrap().push(value.clone());
                if value["toolName"] == "mcp_blocked" || value["toolName"] == "bash" {
                    return Ok(Some(json!({"block":true,"reason":"permission denied"})));
                }
            }
            Ok(None)
        })
        .on_tool(move |_, _| {
            effects2.fetch_add(1, Ordering::SeqCst);
            Ok(AgentToolResult::text("ok"))
        }),
    );
    let (session, _) = build(vec![
        call("c1","codemode",json!({"code":r#"
            text([typeof process, typeof require, typeof fetch, typeof setTimeout]);
            text(await describeTool('mcp_ok'));
            text(await searchTools('mcp'));
            const outcomes = await Promise.allSettled([tools.mcp_ok({}),tools.mcp_blocked({}),tools.mcp__hyphen_name({})]);
            text(outcomes.map(v => v.status));
            try { await tools.bash({command:'touch blocked-by-gate',summary:'gate regression',riskLevel:'low'}); } catch(e) { text(e.message); }
            store('cursor', 42); console.log('saved'); return load('cursor');
        "#})),
        call("c2","codemode",json!({"code":"text(load('cursor')); store('cursor', 99); text('partial'); throw Error('failure');"})),
        call("c3","codemode",json!({"code":"return load('cursor');"})),
        call("c4","codemode",json!({"code":"// @options: {\"timeout_ms\":30}\nwhile(true) {}"})),
        call("c4a","codemode",json!({"code":"// @options: {\"timeout_ms\":30}\ntext('before deadline'); while(true) {}"})),
        call("c5","codemode",json!({"code":"await new Promise(() => {});"})),
        call("c6","codemode",json!({"code":"return typeof tools.codemode;"})),
        FauxResponse::Text("done".into()),
    ], ext, json!({}), dir.path()).await;
    run(&session).await;
    let first = result(&session, "c1");
    assert!(!first.1, "{}", first.0);
    assert!(first.0.contains("undefined"));
    assert!(first.0.contains("rejected"));
    assert!(first.0.contains("saved"));
    assert_eq!(effects.load(Ordering::SeqCst), 2);
    let blocked = hook_calls
        .lock()
        .unwrap()
        .iter()
        .find(|v| v["toolName"] == "mcp_blocked")
        .cloned()
        .unwrap();
    assert_eq!(blocked["parentToolCallId"], "c1");
    assert!(result(&session, "c2").0.contains("partial"));
    assert!(result(&session, "c2").1);
    assert!(result(&session, "c3").0.contains("42"));
    assert!(result(&session, "c4").1);
    assert!(result(&session, "c4a").1);
    assert!(
        result(&session, "c4a").0.contains("before deadline"),
        "{:?}",
        result(&session, "c4a")
    );
    assert!(result(&session, "c5").1);
    assert!(result(&session, "c6").0.contains("undefined"));
    // Every real nested invocation was recorded through the shared dispatcher.
    let nested = session
        .messages()
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) if r.tool_call_id == "c1" => r.nested_calls.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(nested.calls.len(), 4);
    assert!(!dir.path().join("blocked-by-gate").exists());
    let shell = hook_calls
        .lock()
        .unwrap()
        .iter()
        .find(|v| v["toolName"] == "bash")
        .cloned()
        .unwrap();
    assert_eq!(shell["input"]["riskLevel"], "low");
    assert_eq!(shell["parentToolCallId"], "c1");
    session.reload().await.unwrap();
    assert_eq!(session.with_manager(|m| m.branch(None).iter().filter(|e| matches!(&e.kind, oino_session::EntryKind::Custom {custom_type,..} if custom_type == "codemode-store")).count()),1);
}

#[tokio::test]
async fn limits_storage_and_unawaited_calls() {
    let dir = tempfile::tempdir().unwrap();
    let effects = Arc::new(AtomicUsize::new(0));
    let seen = effects.clone();
    let ext = Arc::new(
        FakeExtension::new(
            "mcp",
            Registration {
                tools: vec![spec("mcp_ok")],
                ..Default::default()
            },
        )
        .on_tool(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(AgentToolResult::text("ok"))
        }),
    );
    let (session, _) = build(vec![
        call("l1","codemode",json!({"code":"tools.mcp_ok({}); return 'unawaited';"})),
        call("l2","codemode",json!({"code":"text('before'); exit(); throw Error('after');"})),
        call("l3","codemode",json!({"code":"store('__proto__',{safe:true}); store('x',1); return load('__proto__');"})),
        call("l4","codemode",json!({"code":"text('must not run');\n// @options: {}\nawait tools.mcp_ok({});"})),
        call("l5","codemode",json!({"code":"// @options: {\"max_output_tokens\":10}\nreturn 'A'.repeat(200)+'TAIL';"})),
        call("l6","codemode",json!({"code":"text('partial'); return 'x'.repeat(16777217);"})),
        call("l7","codemode",json!({"code":"for(let n=0;n<100001;n++) text('');"})),
        call("l8","codemode",json!({"code":"try { store('large','x'.repeat(262145)); } catch(e) { text(e.message); } return load('large');"})),
        call("l9","codemode",json!({"code":"return [typeof tools.constructor, typeof tools.__proto__, typeof globalThis.dispatch, typeof models, (await models.getModelsOfType('classifier')).length];"})),
        call("l10","codemode",json!({"code":"text('first'); image('data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII='); text('last');"})),
        FauxResponse::Text("done".into()),
    ],ext,json!({}),dir.path()).await;
    run(&session).await;
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(!result(&session, "l1").1);
    assert!(!result(&session, "l2").1);
    assert!(result(&session, "l2").0.contains("before"));
    assert!(result(&session, "l3").0.contains("safe"));
    assert!(result(&session, "l4").1);
    let truncated = result(&session, "l5");
    assert!(truncated.0.contains("full output:"));
    assert!(truncated.0.contains("TAIL"));
    assert!(
        std::fs::read_dir(dir.path().join("agent/tmp"))
            .unwrap()
            .count()
            >= 1
    );
    assert!(result(&session, "l6").1);
    assert!(result(&session, "l6").0.contains("partial"));
    assert!(result(&session, "l7").1);
    assert!(result(&session, "l8").0.contains("262144"));
    assert!(result(&session, "l9").0.contains("undefined"));
    let image_result = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) if r.tool_call_id == "l10" => Some(r),
            _ => None,
        })
        .unwrap();
    assert!(!image_result.is_error);
    assert!(
        image_result
            .content
            .iter()
            .any(|v| matches!(v, UserContent::Image(_)))
    );
    session.reload().await.unwrap();
    let context = SessionToolContext::default();
    context.bind(&session);
    let store = oino_tools::ToolContext::codemode_store(&context);
    assert_eq!(store["x"], 1);
    assert!(!store.contains_key("large"));
}

#[tokio::test]
async fn only_mode_and_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let mut deferred = spec("deferred_tool");
    deferred.exposure = oino_core::extensions::api::ToolExposure::Deferred;
    let mut hidden = spec("hidden_tool");
    hidden.exposure = oino_core::extensions::api::ToolExposure::Hidden;
    let mut model = spec("model_only");
    model.exposure = oino_core::extensions::api::ToolExposure::ModelOnly;
    let mut code = spec("code_only");
    code.exposure = oino_core::extensions::api::ToolExposure::Codemode;
    let ext = Arc::new(
        FakeExtension::new(
            "test",
            Registration {
                tools: vec![spec("ordinary"), deferred, hidden, model, code],
                ..Default::default()
            },
        )
        .on_tool(|_, _| Ok(AgentToolResult::text("ok"))),
    );
    let (session,_) = build(vec![call("o1","codemode",json!({"code":"text(ALL_TOOLS); text(await tools.ordinary({})); text(await tools.deferred_tool({})); return typeof tools.hidden_tool;"})),FauxResponse::Text("done".into())],ext,json!({"codemode":{"mode":"only"}}),dir.path()).await;
    assert_eq!(session.active_tool_names(), ["codemode", "tool_search"]);
    let description = session
        .agent()
        .with_state(|s| s.tools[0].meta().description.clone());
    assert!(description.contains("declare function ordinary"));
    assert!(!description.contains("declare function deferred_tool"));
    run(&session).await;
    assert!(!result(&session, "o1").1);
    assert!(result(&session, "o1").0.contains("undefined"));
    assert!(
        !session
            .codemode_catalog()
            .iter()
            .any(|v| v["name"] == "hidden_tool" || v["name"] == "model_only")
    );
    assert!(
        session
            .search_deferred_tools("select:hidden_tool", None)
            .is_err()
    );
}
