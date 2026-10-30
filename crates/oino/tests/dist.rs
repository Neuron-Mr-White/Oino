//! End-to-end distribution tests (WP-C10, dist.md §5): no bundle in the binary; the share
//! dir feeds the packages (`OINO_DIST_EXTENSIONS`), user copies shadow by id, and the docs
//! root chain (`OINO_DOCS_ROOT` > `<share root>/docs` > legacy `~/.oino/bundled/oino.docs@*`
//! > absent with the docs section omitted) holds.
//!
//! Print, JSON and ACP modes answer through the `provider-faux` fixture package.
//!
//! Skips (passes without running) when `extensions/dist` is missing (`cargo xtask wasm`).

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
    let dist_built = dist().join("unipi.core").join("component.wasm").is_file();
    (package.join("component.wasm").is_file() && dist_built).then_some(package)
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("oino-dist-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        for sub in [
            "home",
            "agent/cache",
            "work",
            "share/extensions",
            "share/docs",
        ] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        // The staged share tree: every distribution package plus the docs tree (P-DOCS).
        if let Ok(entries) = std::fs::read_dir(dist()) {
            for entry in entries.filter_map(Result::ok) {
                let from = entry.path();
                if from.join("component.wasm").is_file() {
                    copy_dir(
                        &from,
                        &dir.path().join("share/extensions").join(entry.file_name()),
                    );
                }
            }
        }
        std::fs::write(dir.path().join("share/docs/README.md"), "# staged docs\n").unwrap();
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

    fn share(&self) -> PathBuf {
        self.dir.path().join("share/extensions")
    }

    fn command(&self, package: &Path, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut command = self.bare(args, envs);
        command
            .arg("-e")
            .arg(package)
            .args(["--model", "faux/faux-1"]);
        command
    }

    /// A command without the `-e`/`--model` prefix (package subcommands take no flags).
    fn bare(&self, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_oino"));
        command
            .current_dir(self.dir.path().join("work"))
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.dir.path().join("home"))
            .env("OINO_AGENT_DIR", self.dir.path().join("agent"))
            .env("OINO_DIST_EXTENSIONS", self.share())
            .envs(envs.iter().copied())
            .args(args);
        command
    }

    fn run(&self, package: &Path, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
        self.command(package, args, envs)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn run_bare(&self, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
        self.bare(args, envs).stdin(Stdio::null()).output().unwrap()
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn binary_carries_no_bundled_components() {
    // P-BUNDLE: the embedded table and its env knob are gone from the binary (strings check).
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_oino")).unwrap();
    assert!(
        !bytes
            .windows(b"OINO_BUNDLED".len())
            .any(|w| w == b"OINO_BUNDLED"),
        "OINO_BUNDLED must not appear in the binary"
    );
    assert!(
        !bytes
            .windows(b"oino-bundle".len())
            .any(|w| w == b"oino-bundle"),
        "the oino-bundle crate must not be linked into the binary"
    );
}

/// The system message sections of a `--mode json` run.
fn system_sections(sandbox: &Sandbox, package: &Path, envs: &[(&str, &str)]) -> Value {
    let output = sandbox.run(package, &["--mode", "json", "hi"], envs);
    assert!(output.status.success(), "{}", stderr(&output));
    let events: Vec<Value> = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let system = events
        .iter()
        .find(|e| e["type"] == "message_end" && e["message"]["role"] == "system")
        .expect("system message");
    system["message"]["sections"].clone()
}

#[test]
fn share_dir_feeds_packages_and_docs_root() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    // Print run through the share dir (plus the `-e` faux provider).
    let output = sandbox.run(&package, &["-p", "hi"], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Hello from faux.\n");

    // The share packages load and the docs section points at the staged docs tree.
    let sections = system_sections(&sandbox, &package, &[]);
    let tools = sections["tools"].as_str().unwrap();
    for tool in ["bg_run", "memory_search", "web_search", "plan_submit"] {
        assert!(tools.contains(tool), "{tool} missing from {tools}");
    }
    let docs = sections["docs"].as_str().unwrap();
    let staged = sandbox.dir.path().join("share/docs").display().to_string();
    assert!(
        docs.contains(&format!("{staged}/README.md")),
        "docs section must point at the share docs tree: {docs}"
    );

    // `oino list` shows the distribution packages; the override emptied disables them.
    let list = sandbox.run_bare(&["list"], &[]);
    assert_eq!(list.status.code(), Some(0), "{}", stderr(&list));
    let listing = stdout(&list);
    for id in ["unipi.core", "oino.anthropic"] {
        let line = listing
            .lines()
            .find(|l| l.starts_with(id))
            .unwrap_or_else(|| panic!("{id} missing from {listing}"));
        assert!(line.contains("dist"), "{line}");
    }
    let emptied = sandbox.run_bare(&["list"], &[("OINO_DIST_EXTENSIONS", "")]);
    assert!(!stdout(&emptied).contains("unipi.core"));

    // A user copy of the same id shadows the distribution one (dist.md §5).
    copy_dir(
        &sandbox.share().join("unipi.notify"),
        &sandbox.dir.path().join("agent/extensions/unipi.notify"),
    );
    let listing = stdout(&sandbox.run_bare(&["list"], &[]));
    let notify: Vec<&str> = listing
        .lines()
        .filter(|l| l.starts_with("unipi.notify"))
        .collect();
    assert_eq!(
        notify.len(),
        2,
        "the shadowed id is listed once per source (dist + global): {listing}"
    );
    assert!(notify.iter().any(|l| l.contains("global")));
}

#[test]
fn docs_root_chain_env_share_legacy_absent() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    // OINO_DOCS_ROOT wins over the share tree.
    let env_docs = sandbox.dir.path().join("env-docs");
    std::fs::create_dir_all(&env_docs).unwrap();
    let sections = system_sections(
        &sandbox,
        &package,
        &[("OINO_DOCS_ROOT", env_docs.to_str().unwrap())],
    );
    assert!(
        sections["docs"]
            .as_str()
            .unwrap()
            .contains(env_docs.display().to_string().as_str()),
        "{}",
        sections["docs"]
    );

    // Without a share docs tree, the legacy extraction of retired bundles is used.
    std::fs::remove_dir_all(sandbox.dir.path().join("share/docs")).unwrap();
    let legacy = sandbox
        .dir
        .path()
        .join("agent/bundled/oino.docs@9.9.9-aaaabbbb");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("README.md"), "# legacy\n").unwrap();
    let sections = system_sections(&sandbox, &package, &[]);
    assert!(
        sections["docs"]
            .as_str()
            .unwrap()
            .contains("oino.docs@9.9.9"),
        "{}",
        sections["docs"]
    );

    // Absent: the section is omitted instead of pointing at a missing path (P-DOCS).
    std::fs::remove_dir_all(legacy.parent().unwrap()).unwrap();
    let sections = system_sections(&sandbox, &package, &[]);
    assert!(
        sections.as_object().unwrap().contains_key("preamble"),
        "the prompt is still built: {sections}"
    );
    assert!(
        !sections.as_object().unwrap().contains_key("docs"),
        "the docs section must be omitted when the root is absent: {sections}"
    );
}

#[test]
fn acp_round_trip_through_the_share_dir() {
    let Some(package) = faux_package() else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: run `cargo xtask wasm` to build extensions/dist"
        );
        return;
    };
    let sandbox = Sandbox::new();

    let mut child = sandbox
        .command(&package, &["--mode", "acp"], &[])
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
    let wait = |id: u64, seen: &mut Vec<Value>| -> Value {
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
    assert_ne!(text, "");
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());
}
