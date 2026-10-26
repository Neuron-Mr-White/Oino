//! `oino --mode acp` applies `--model` / `--thinking` to new ACP sessions (STATUS known
//! issue 5): the `session/new` config options report the CLI choice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use support::Sandbox;

fn current(options: &Value, id: &str) -> Value {
    options
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .map_or_else(
            || panic!("no {id} option in {options:#}"),
            |o| o["currentValue"].clone(),
        )
}

fn new_session_options(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--mode", "acp"];
    all.extend_from_slice(args);
    let mut child = sandbox
        .command(&json!([]), &all)
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
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && tx.send(value).is_err()
            {
                break;
            }
        }
    });
    let wait = |id: u64| loop {
        let value = rx.recv_timeout(Duration::from_secs(60)).expect("ACP reply");
        if value["id"] == id {
            return value;
        }
    };
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})
    )
    .unwrap();
    wait(1);
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd": sandbox.cwd(),"mcpServers":[]}})
    )
    .unwrap();
    let reply = wait(2);
    drop(stdin);
    let _ = child.wait();
    reply["result"]["configOptions"].clone()
}

#[test]
fn cli_model_and_thinking_reach_acp_sessions() {
    let sandbox = Sandbox::new();
    let options = new_session_options(&sandbox, &["--model", "faux/faux-1:high"]);
    assert_eq!(current(&options, "model"), "faux/faux-1");
    assert_eq!(current(&options, "thinking"), "high");
    let options = new_session_options(
        &sandbox,
        &[
            "--provider",
            "faux",
            "--model",
            "faux-1",
            "--thinking",
            "low",
        ],
    );
    assert_eq!(current(&options, "thinking"), "low");
}
