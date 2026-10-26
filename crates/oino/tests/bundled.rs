//! End-to-end smoke with the bundled components: print, JSON and ACP modes load every
//! bundled provider and unipi component and answer through the `provider-faux` fixture
//! package (`extensions/dist`, built by `cargo xtask wasm`).
//!
//! Skips (passes without running) when `extensions/dist` or the bundle is missing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/dist")
}

fn faux_package() -> Option<PathBuf> {
    let package = dist().join("oino.fixture.provider-faux");
    let bundled = dist().join("unipi.core").join("component.wasm").is_file();
    (package.join("component.wasm").is_file() && bundled).then_some(package)
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("oino-bundled-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        for sub in ["home", "agent/cache", "work"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        // Share the wasmtime compile cache between runs (the first compile is slow in debug).
        let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
        std::fs::create_dir_all(&cache).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&cache, dir.path().join("agent/cache/wasmtime")).unwrap();
        std::fs::write(
            dir.path().join("agent/auth.json"),
            r#"{"faux":{"type":"api_key","key":"k"}}"#,
        )
        .unwrap();
        Self { dir }
    }

    fn command(&self, package: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_oino"));
        command
            .current_dir(self.dir.path().join("work"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("OINO_AGENT_DIR", self.dir.path().join("agent"))
            .arg("-e")
            .arg(package)
            .args(["--model", "faux/faux-1"])
            .args(args);
        command
    }
}

#[test]
fn print_json_and_acp_run_with_the_bundle() {
    let Some(package) = faux_package() else {
        eprintln!("skipped: run `cargo xtask wasm` to build extensions/dist");
        return;
    };
    let sandbox = Sandbox::new();

    let output = sandbox.command(&package, &["-p", "hi"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Hello from faux.\n"
    );
    let bundled = std::fs::read_dir(sandbox.dir.path().join("agent/bundled"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for prefix in [
        "oino.anthropic@",
        "unipi.core@",
        "unipi.workflow@",
        "oino.docs@",
    ] {
        assert!(
            bundled.iter().any(|name| name.starts_with(prefix)),
            "{prefix} in {bundled:?}"
        );
    }

    let output = sandbox
        .command(&package, &["--mode", "json", "hi"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let events: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let system = events
        .iter()
        .find(|e| e["type"] == "message_end" && e["message"]["role"] == "system")
        .expect("system message");
    let tools = system["message"]["sections"]["tools"].as_str().unwrap();
    for tool in ["bg_run", "memory_search", "web_search", "plan_submit"] {
        assert!(tools.contains(tool), "{tool} missing from {tools}");
    }
    let docs = system["message"]["sections"]["docs"].as_str().unwrap();
    assert!(docs.contains("oino.docs@"), "{docs}");
    assert!(events.iter().any(|e| e["type"] == "agent_settled"));

    acp_round_trip(&sandbox, &package);
}

fn acp_round_trip(sandbox: &Sandbox, package: &Path) {
    let mut child = sandbox
        .command(package, &["--mode", "acp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str(&line)
                && tx.send(value).is_err()
            {
                break;
            }
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, value: Value| {
        writeln!(stdin, "{value}").unwrap();
    };
    let mut seen = Vec::new();
    let mut wait = |id: u64, seen: &mut Vec<Value>| -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(found) = seen.iter().find(|v| v["id"] == id) {
                return found.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            seen.push(rx.recv_timeout(left).expect("ACP response"));
        }
    };
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}),
    );
    assert_eq!(wait(1, &mut seen)["result"]["agentInfo"]["name"], "oino");
    let cwd = sandbox.dir.path().join("work");
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd": cwd,"mcpServers":[]}}),
    );
    let session = wait(2, &mut seen)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":3,"method":"session/set_config_option","params":{"sessionId":session,"configId":"model","value":"faux/faux-1"}}),
    );
    let _ = wait(3, &mut seen);
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"hi"}]}}),
    );
    assert_eq!(wait(4, &mut seen)["result"]["stopReason"], "end_turn");
    let text: String = seen
        .iter()
        .filter(|v| v["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|v| v["params"]["update"]["content"]["text"].as_str())
        .collect();
    assert!(!text.is_empty());
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());
}
