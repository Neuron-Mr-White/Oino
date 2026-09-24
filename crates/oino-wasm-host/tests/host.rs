//! Acceptance tests for the wasmtime component host: tool/command/hook
//! round-trips, capability denials, limits, provider streaming, cache reuse.

// Test assertions unwrap liberally; the workspace lint prefers explicit
// `unwrap_or_else(|err| panic!(...))` but 80+ mechanical sites add no value.
#![allow(clippy::unwrap_used)]

use oino_agent_loop::StreamProvider;
use oino_extension_core::{ExtensionPermissions, PersistenceScope};
use oino_types::{AssistantStreamEvent, ContentBlock, OinoId};
use oino_wasm_host::{
    CallLimits, HostCaps, SecretResolver, WasmExtension, WasmHostEngine, WasmHostError,
    WasmStreamProvider,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fixture.wasm"
    ))
    .unwrap_or_else(|err| panic!("fixture.wasm: {err}"))
}

fn provider_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fixture_provider.wasm"
    ))
    .unwrap_or_else(|err| panic!("fixture_provider.wasm: {err}"))
}

fn caps(permissions: ExtensionPermissions) -> HostCaps {
    HostCaps {
        permissions,
        kv_dir: tempfile::tempdir()
            .unwrap_or_else(|err| panic!("kv dir: {err}"))
            .keep(),
        ..HostCaps::default()
    }
}

async fn extension(permissions: ExtensionPermissions) -> (Arc<WasmHostEngine>, WasmExtension) {
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep())
        .unwrap_or_else(|err| panic!("engine: {err}"));
    let extension = WasmExtension::new(
        engine.clone(),
        &fixture_bytes(),
        caps(permissions),
        CallLimits::default(),
        false,
    )
    .unwrap_or_else(|err| panic!("extension: {err}"));
    (engine, extension)
}

fn full_permissions() -> ExtensionPermissions {
    ExtensionPermissions {
        tools: BTreeSet::from(["*".to_string()]),
        commands: BTreeSet::from(["*".to_string()]),
        session_persistence: BTreeSet::from([PersistenceScope::Project]),
        ..Default::default()
    }
}

#[tokio::test]
async fn init_returns_registration() {
    let (_engine, extension) = extension(full_permissions()).await;
    let registration = extension
        .init(json!({}))
        .await
        .unwrap_or_else(|err| panic!("init: {err}"));
    let tools = registration["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools"));
    assert!(tools.iter().any(|tool| tool["name"] == "echo"));
    assert!(registration["commands"].as_array().unwrap().len() >= 8);
    assert_eq!(registration["hooks"], json!(["input"]));
}

#[tokio::test]
async fn tool_round_trip_emits_progress() {
    let progress: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let progress = Arc::clone(&progress);
        Arc::new(
            move |call_id: &str, message: &str, _details: Option<String>| {
                progress
                    .lock()
                    .unwrap()
                    .push((call_id.to_string(), message.to_string()));
            },
        )
    };
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let mut host_caps = caps(full_permissions());
    host_caps.progress = sink;
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        host_caps,
        CallLimits::default(),
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let call_id = OinoId::new_v4();
    let output = extension
        .run_tool("echo", call_id, json!({"hello": "world"}))
        .await
        .unwrap_or_else(|err| panic!("run_tool: {err}"));
    assert!(!output.is_error);
    match &output.content[0] {
        ContentBlock::Text { text } => assert!(text.contains("hello"), "{text}"),
        _ => panic!("expected text"),
    }
    let progress = progress.lock().unwrap();
    assert!(progress
        .iter()
        .any(|(id, msg)| *id == call_id.to_string() && msg == "echo started"));
    assert!(progress.iter().any(|(_, msg)| msg == "echo done"));
}

#[tokio::test]
async fn command_and_hook_round_trip() {
    let (_engine, extension) = extension(full_permissions()).await;
    extension.init(json!({})).await.unwrap();
    let hello = extension.run_command("hello", "there").await.unwrap();
    assert_eq!(hello["greeting"], "hello there");
    let upper = extension
        .handle_event("input", json!({"text": "shout"}))
        .await
        .unwrap();
    assert_eq!(upper["text"], "SHOUT");
}

#[tokio::test]
async fn kv_persists_across_recreation() {
    let dir = tempfile::tempdir().unwrap();
    let kv_dir = dir.keep();
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let mut host_caps = caps(full_permissions());
    host_caps.kv_dir = kv_dir;
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        host_caps,
        CallLimits::default(),
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    extension.run_command("kv-set", "k1 v1").await.unwrap();
    // Force re-instantiation via a trap (spin hits the deadline).
    let err = extension.run_command("spin", "").await.unwrap_err();
    assert!(matches!(err, WasmHostError::Deadline), "{err:?}");
    let value = extension.run_command("kv-get", "k1").await.unwrap();
    assert_eq!(value["value"], "v1");
}

#[tokio::test]
async fn spin_interrupted_by_deadline_and_host_usable() {
    let (_engine, extension) = extension(full_permissions()).await;
    extension.init(json!({})).await.unwrap();
    let limits = CallLimits {
        command: Duration::from_millis(150),
        ..CallLimits::default()
    };
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let spin_ext = WasmExtension::new(
        engine,
        &fixture_bytes(),
        caps(full_permissions()),
        limits,
        false,
    )
    .unwrap();
    spin_ext.init(json!({})).await.unwrap();
    let err = spin_ext.run_command("spin", "").await.unwrap_err();
    assert!(matches!(err, WasmHostError::Deadline), "{err:?}");
    // other extension keeps working
    let hello = extension.run_command("hello", "after").await.unwrap();
    assert_eq!(hello["greeting"], "hello after");
}

#[tokio::test]
async fn hog_traps_under_memory_cap() {
    let limits = CallLimits {
        memory_bytes: 32 * 1024 * 1024,
        ..CallLimits::default()
    };
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        caps(full_permissions()),
        limits,
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let err = extension.run_command("hog", "").await.unwrap_err();
    assert!(
        matches!(err, WasmHostError::Instantiate(_) | WasmHostError::Deadline),
        "{err:?}"
    );
}

#[tokio::test]
async fn unhealthy_after_repeated_traps() {
    let limits = CallLimits {
        command: Duration::from_millis(100),
        max_restarts: 1,
        ..CallLimits::default()
    };
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        caps(full_permissions()),
        limits,
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    for _ in 0..3 {
        let _ = extension.run_command("spin", "").await;
    }
    let err = extension.run_command("hello", "x").await.unwrap_err();
    assert!(matches!(err, WasmHostError::Unhealthy(_)), "{err:?}");
}

fn http_server(response: &'static str) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok(pair) => break pair.0,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() > deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        };
        let mut request = String::new();
        let _ = BufReader::new(&mut stream).read_line(&mut request);
        let _ = stream.write_all(response.as_bytes());
    });
    (port, handle)
}

#[tokio::test]
async fn undeclared_host_denied() {
    let (port, handle) = http_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
    let permissions = ExtensionPermissions {
        network: oino_extension_core::NetworkPermission {
            raw_network: false,
            hosts: vec!["example.com".into()],
        },
        ..full_permissions()
    };
    let (_engine, extension) = extension(permissions).await;
    extension.init(json!({})).await.unwrap();
    let err = extension
        .run_command("http-get", &format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap_err();
    assert!(matches!(err, WasmHostError::Guest(_)), "{err:?}");
    handle.join().ok();
}

#[tokio::test]
async fn declared_host_allowed() {
    let (port, handle) = http_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
    let permissions = ExtensionPermissions {
        network: oino_extension_core::NetworkPermission {
            raw_network: false,
            hosts: vec!["127.0.0.1".into()],
        },
        ..full_permissions()
    };
    let (_engine, extension) = extension(permissions).await;
    extension.init(json!({})).await.unwrap();
    let result = extension
        .run_command("http-get", &format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap_or_else(|err| panic!("http-get: {err}"));
    assert_eq!(result["status"], 200, "{result}");
    assert_eq!(result["body"], "hi", "{result}");
    handle.join().ok();
}

#[tokio::test]
async fn undeclared_secret_denied() {
    let (_engine, extension) = extension(full_permissions()).await;
    extension.init(json!({})).await.unwrap();
    let result = extension.run_command("secret", "token").await.unwrap();
    let err = result["value"]["Err"].as_str().unwrap_or_default();
    assert!(err.contains("capability denied"), "{result}");
}

struct TestSecrets;
impl SecretResolver for TestSecrets {
    fn secret(&self, name: &str) -> Option<String> {
        (name == "token").then(|| "hunter2".to_string())
    }
}

#[tokio::test]
async fn declared_secret_resolves() {
    let mut permissions = full_permissions();
    permissions.secrets = BTreeSet::from(["token".to_string()]);
    let mut host_caps = caps(permissions);
    host_caps.secrets = Arc::new(TestSecrets);
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        host_caps,
        CallLimits::default(),
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let result = extension.run_command("secret", "token").await.unwrap();
    assert_eq!(result["value"]["Ok"], "hunter2", "{result}");
}

#[tokio::test]
async fn exec_denied_without_permission() {
    let (_engine, extension) = extension(full_permissions()).await;
    extension.init(json!({})).await.unwrap();
    let err = extension.run_command("exec", "echo hi").await.unwrap_err();
    assert!(matches!(err, WasmHostError::Guest(_)), "{err:?}");
}

#[tokio::test]
async fn exec_allowed_for_declared_command() {
    let mut permissions = full_permissions();
    permissions.shell_process = oino_extension_core::ShellProcessPermission {
        allowed: true,
        commands: vec!["echo".into()],
    };
    let (_engine, extension) = extension(permissions).await;
    extension.init(json!({})).await.unwrap();
    let result = extension.run_command("exec", "echo hi").await.unwrap();
    assert_eq!(result["status"], 0);
    assert_eq!(result["stdout"], "hi\n");
}

#[tokio::test]
async fn undeclared_path_denied() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "s3cret").unwrap();
    let (_engine, extension) = extension(full_permissions()).await;
    extension.init(json!({})).await.unwrap();
    let err = extension
        .run_command("read-file", &format!("{}/secret.txt", dir.path().display()))
        .await
        .unwrap_err();
    assert!(matches!(err, WasmHostError::Guest(_)), "{err:?}");
}

#[tokio::test]
async fn declared_path_reads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.txt"), "contents").unwrap();
    let mut permissions = full_permissions();
    permissions.filesystem = vec![oino_extension_core::FilesystemPermission {
        access: oino_extension_core::FilesystemAccess::Read,
        paths: vec![dir.path().to_string_lossy().into_owned()],
    }];
    let (_engine, extension) = extension(permissions).await;
    extension.init(json!({})).await.unwrap();
    let result = extension
        .run_command("read-file", &format!("{}/data.txt", dir.path().display()))
        .await
        .unwrap_or_else(|err| panic!("read-file: {err}"));
    assert_eq!(result["text"], "contents");
}

#[tokio::test]
async fn provider_streams_10k_events() {
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &provider_bytes(),
        caps(full_permissions()),
        CallLimits::default(),
        true,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let provider = WasmStreamProvider::new(Arc::new(extension)).unwrap();
    let request = faux_request(json!({"faux_events": 10_000}));
    let signal = oino_agent_loop::AbortSignal::new();
    let events = provider
        .stream(request, signal)
        .await
        .unwrap_or_else(|err| panic!("stream: {err}"));
    assert_eq!(events.len(), 10_000);
    for (index, event) in events.iter().enumerate() {
        match event {
            AssistantStreamEvent::TextDelta { delta } => {
                assert_eq!(delta, &format!("event-{index}"));
            }
            _ => panic!("unexpected event {event:?}"),
        }
    }
}

#[tokio::test]
async fn provider_sse_mode() {
    let (port, handle) = http_server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 33\r\n\r\ndata: one\ndata: two\ndata: three\n\n",
    );
    let permissions = ExtensionPermissions {
        network: oino_extension_core::NetworkPermission {
            raw_network: false,
            hosts: vec!["127.0.0.1".into()],
        },
        ..full_permissions()
    };
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &provider_bytes(),
        caps(permissions),
        CallLimits::default(),
        true,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let provider = WasmStreamProvider::new(Arc::new(extension)).unwrap();
    let request = faux_request(json!({"sse_url": format!("http://127.0.0.1:{port}/")}));
    let signal = oino_agent_loop::AbortSignal::new();
    let events = provider
        .stream(request, signal)
        .await
        .unwrap_or_else(|err| panic!("sse: {err}"));
    let texts: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            AssistantStreamEvent::TextDelta { delta } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["one", "two", "three"]);
    handle.join().ok();
}

#[tokio::test]
async fn warm_start_uses_cache() {
    let dir = tempfile::tempdir().unwrap().keep();
    let bytes = fixture_bytes();
    let engine = WasmHostEngine::new(&dir).unwrap();
    let _component = engine.component(&bytes).unwrap();
    // cache dir must contain compiled artifacts after first compile
    fn has_files(dir: &std::path::Path) -> bool {
        std::fs::read_dir(dir)
            .map(|mut e| e.next().is_some())
            .unwrap_or(false)
            || std::fs::read_dir(dir).unwrap().any(|e| {
                e.map(|e| e.path().is_dir() && has_files(&e.path()))
                    .unwrap_or(false)
            })
    }
    assert!(has_files(&dir), "cache dir empty after compile");
    // a second engine loading the same bytes completes (cache hit path)
    let engine2 = WasmHostEngine::new(&dir).unwrap();
    let _component2 = engine2.component(&bytes).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "bench"]
async fn bench_overhead() {
    let engine = WasmHostEngine::new(tempfile::tempdir().unwrap().keep()).unwrap();
    let extension = WasmExtension::new(
        engine,
        &fixture_bytes(),
        caps(full_permissions()),
        CallLimits::default(),
        false,
    )
    .unwrap();
    extension.init(json!({})).await.unwrap();
    let mut times = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = std::time::Instant::now();
        let _ = extension.run_command("hello", "x").await.unwrap();
        times.push(start.elapsed());
    }
    times.sort();
    let p95 = times[949];
    println!("p95 per call: {p95:?} (target < 1ms)");
    assert!(p95 < Duration::from_millis(5), "p95 too high: {p95:?}");
}

fn faux_request(payload: Value) -> oino_agent_loop::StreamRequest {
    use oino_agent_loop::CacheRetention;
    use oino_types::{Model, ThinkingLevel};
    oino_agent_loop::StreamRequest {
        model: Model::new("faux", "faux-1"),
        thinking_level: ThinkingLevel::Off,
        transcript: oino_types::transcript::Transcript::new(Vec::new()),
        session_id: Some(payload.to_string()),
        cache_retention: CacheRetention::None,
    }
}
