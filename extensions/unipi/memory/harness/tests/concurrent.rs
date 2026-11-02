//! fix-frames P0: `unipi.memory` trapped ("component instance stopped", wasm `unreachable`)
//! whenever two calls met on its one `mempalace-mcp` reader — the owner's agent ran
//! `memory_search` and `global_memory_search` in parallel; a hub eviction's
//! `session_shutdown` killed the reader while a search held it. The reader is behind an
//! async `Mutex`; the waiting call is a *different component-model task*, and waking it
//! needs wit-bindgen's `inter-task-wakeup` feature. Without it the guest panicked
//! (`logs/ext/unipi.memory.log`: "Cannot support cross-component-model-task wakeup unlses
//! the `wit-bindgen` crate is compiled with the `inter-task-wakeup` feature enabled").
//!
//! Here a fake `mempalace` / `mempalace-mcp` (a slow stdio JSON-RPC server) puts the
//! extension in palace mode, and parallel searches plus a shutdown during a search must
//! all answer — no trap.
//!
//! The test points `HOME` at a temp dir first: the extension's `~/.mempalace` preopen then
//! never reaches the owner's real palace.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use oino_extension_api::manifest::Manifest;
use oino_extension_api::ops::SettingsScope;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson, ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds the component (release, wasm32-wasip2) with the real toolchain homes.
fn build(cargo_home: &Path, rustup_home: &Path, home: &Path) -> PathBuf {
    let ws =
        std::env::var("UNIPI_MEMORY_WORKSPACE").map_or_else(|_| extensions_dir(), PathBuf::from);
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(&ws)
        .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"])
        .args(["-p", "unipi-memory"]);
    for var in [
        "CARGO_TARGET_DIR",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_MANIFEST_DIR",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"))
        .env("CARGO_HOME", cargo_home)
        .env("RUSTUP_HOME", rustup_home)
        .env("HOME", home);
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    extensions_dir().join("target/wasm32-wasip2/release/unipi_memory.wasm")
}

/// `mempalace --version` and a `mempalace-mcp` that answers every search after 400 ms.
fn fake_mempalace(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let cli = dir.join("mempalace");
    std::fs::write(&cli, "#!/bin/sh\necho 'mempalace 3.11.0'\n").unwrap();
    let mcp = dir.join("mempalace-mcp");
    std::fs::write(
        &mcp,
        r#"#!/usr/bin/env python3
import json, sys, time
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    if "id" not in msg:
        continue
    if msg.get("method") == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "fake", "version": "1"}}
    else:
        time.sleep(0.4)
        query = msg["params"]["arguments"].get("query", "")
        payload = {"results": [{"content": "remembered " + query, "source_file": "/elsewhere/" + query.replace(" ", "_") + ".md", "score": 0.9, "metadata": {"wing": "unipi", "room": "decision"}}]}
        result = {"content": [{"type": "text", "text": json.dumps(payload)}]}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}) + "\n")
    sys.stdout.flush()
"#,
    )
    .unwrap();
    for f in [&cli, &mcp] {
        std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    cli
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn cx(cwd: &Path, kind: CallKind, session: &str) -> CallContext {
    CallContext {
        session_id: session.into(),
        mode: Mode::Rpc,
        has_ui: false,
        cwd: cwd.to_path_buf(),
        kind,
        ..CallContext::default()
    }
}

async fn search(ext: &Arc<dyn ExtensionInstance>, cwd: &Path, tool: &str, query: &str) -> String {
    let inv = ToolInvocation {
        tool_call_id: format!("c-{tool}-{query}"),
        name: tool.into(),
        args: RawJson::from(json!({"query": query, "limit": 3})),
        parent_tool_call_id: None,
    };
    match ext
        .run_tool(inv, CancellationToken::new(), cx(cwd, CallKind::Tool, "s1"))
        .await
    {
        Ok(result) => {
            let value: Value = result.to_value();
            value["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        }
        Err(error) => format!("ERROR: {error}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_searches_and_a_shutdown_mid_search_never_trap() {
    // Isolate HOME (the `~/.mempalace` grant) before anything reads it; the build keeps the
    // real toolchain.
    let real_home = PathBuf::from(std::env::var("HOME").unwrap());
    let cargo_home =
        std::env::var("CARGO_HOME").map_or_else(|_| real_home.join(".cargo"), PathBuf::from);
    let rustup_home =
        std::env::var("RUSTUP_HOME").map_or_else(|_| real_home.join(".rustup"), PathBuf::from);
    let home = tempfile::tempdir().unwrap();
    let wasm = build(&cargo_home, &rustup_home, &real_home);
    // SAFETY: this test binary runs this one test; nothing else reads the environment now.
    unsafe { std::env::set_var("HOME", home.path()) };

    let bin = tempfile::tempdir().unwrap();
    let cli = fake_mempalace(bin.path());
    let agent = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let cwd = project.path().join("unipi");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
    options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
    let runtime = WasmRuntime::new(options).unwrap();
    let recording = RecordingHostServices::new();
    // The extension re-reads its settings from the host (`extension_settings`): the fake
    // binary must be configured there, or the real `mempalace` on PATH would be used.
    for (key, value) in [
        ("mempalaceBin", json!(cli.display().to_string())),
        ("mempalaceAutoUpdate", json!(false)),
        ("recallAtStart", json!(false)),
        ("saveMode", json!("off")),
        ("write", json!(false)),
    ] {
        recording.preset_extension_setting(SettingsScope::Global, "memory", key, value);
    }
    let host: Arc<dyn HostServices> = Arc::clone(&recording) as Arc<dyn HostServices>;
    let manifest = Manifest::parse(
        &std::fs::read_to_string(extensions_dir().join("unipi/memory/extension.json")).unwrap(),
    )
    .unwrap();
    let ext = runtime
        .load(
            LoadedPackage {
                manifest,
                component: ComponentSource::Path(wasm),
                root: None,
                bundled: true,
            },
            host,
        )
        .await
        .unwrap();
    let settings = json!({"memory": {
        "write": false, "recallAtStart": false, "saveMode": "off",
        "mempalaceAutoUpdate": false, "debug": true, "mempalaceBin": cli.display().to_string()
    }});
    ext.init(
        InitInput {
            settings: RawJson::from(settings),
            ..InitInput::default()
        },
        cx(&cwd, CallKind::Init, "s1"),
    )
    .await
    .unwrap();
    // `session_start` detects the (fake) MemPalace in the background: wait for palace mode.
    for session in ["s1", "s2"] {
        ext.on_event(
            EventKind::SessionStart,
            RawJson::from(json!({"type": "session_start", "reason": "startup"})),
            cx(&cwd, CallKind::Event, session),
        )
        .await
        .unwrap();
    }
    let mut warm = String::new();
    for _ in 0..50 {
        warm = search(&ext, &cwd, "memory_search", "warm up").await;
        if warm.starts_with("Found") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if !warm.starts_with("Found") {
        let mut logs = String::new();
        for entry in walk(agent.path()) {
            if entry.extension().is_some_and(|e| e == "log") {
                logs.push_str(&format!("== {}\n", entry.display()));
                logs.push_str(&std::fs::read_to_string(&entry).unwrap_or_default());
            }
        }
        let files: Vec<String> = walk(agent.path())
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let calls: Vec<String> = recording
            .calls()
            .iter()
            .map(|c| format!("{c:?}").chars().take(300).collect())
            .collect();
        panic!("palace mode never came up: {warm}\n{logs}\nfiles: {files:#?}\ncalls: {calls:#?}");
    }

    // The owner's turn: `memory_search` and `global_memory_search` at once (plus a third).
    let (a, b, c) = tokio::join!(
        search(&ext, &cwd, "memory_search", "tool test harness"),
        search(&ext, &cwd, "global_memory_search", "browser"),
        search(&ext, &cwd, "global_memory_search", "agent37"),
    );
    for (query, text) in [("tool test harness", &a), ("browser", &b), ("agent37", &c)] {
        if text.contains("component instance stopped") {
            // The SDK's panic hook logs the guest panic through the host's `log` import
            // (host tracing + this file) before the trap.
            let log = std::fs::read_to_string(agent.path().join("logs/ext/unipi.memory.log"))
                .unwrap_or_default();
            panic!("trapped: {text}\n{log}");
        }
        assert!(text.starts_with("Found"), "{query}: {text}");
        assert!(text.contains(&query.replace(' ', "_")), "{query}: {text}");
    }

    // A hub eviction: `session_shutdown` of another session kills the shared reader while a
    // search holds it.
    let shutdown = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        ext.on_event(
            EventKind::SessionShutdown,
            RawJson::from(json!({"type": "session_shutdown"})),
            cx(&cwd, CallKind::Event, "s2"),
        )
        .await
    };
    let (text, shut) = tokio::join!(
        search(&ext, &cwd, "memory_search", "during shutdown"),
        shutdown
    );
    assert!(shut.is_ok(), "{shut:?}");
    assert!(text.starts_with("Found"), "{text}");
    // And the instance still answers afterwards (a fresh reader).
    let after = search(&ext, &cwd, "memory_search", "after").await;
    assert!(after.starts_with("Found"), "{after}");
}
