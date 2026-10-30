//! Shared harness: an isolated home/agent dir, the `oino` binary with the faux provider,
//! and a line-oriented RPC driver over the child's stdio.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness shared by several test crates"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(20);

/// The `oino` binary under test.
pub(crate) fn oino_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_oino"))
}

/// A temp dir with `home/`, `agent/` and `work/` (the cwd).
pub(crate) struct Sandbox {
    pub(crate) dir: tempfile::TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["home", "agent", "work"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    pub(crate) fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub(crate) fn agent(&self) -> PathBuf {
        self.dir.path().join("agent")
    }

    pub(crate) fn cwd(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    pub(crate) fn write_settings(&self, settings: &Value) {
        std::fs::write(self.agent().join("settings.json"), settings.to_string()).unwrap();
    }

    /// `oino` with the faux provider scripted by `responses`.
    pub(crate) fn command(&self, responses: &Value, args: &[&str]) -> Command {
        let mut command = Command::new(oino_bin());
        command
            .current_dir(self.cwd())
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.home())
            .env("OINO_AGENT_DIR", self.agent())
            .env("OINO_FAUX_API_KEY", "faux-key")
            .env("OINO_FAUX_RESPONSES", responses.to_string())
            .args(["-e", "builtin:faux", "-nc", "-ns", "-np"])
            .args(args);
        command
    }

    /// Runs to completion with `stdin` piped in.
    pub(crate) fn run(&self, responses: &Value, args: &[&str], stdin: Option<&str>) -> Output {
        let mut command = self.command(responses, args);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        {
            let mut input = child.stdin.take().unwrap();
            if let Some(text) = stdin {
                input.write_all(text.as_bytes()).unwrap();
            }
        }
        child.wait_with_output().unwrap()
    }

    /// Starts `oino --mode rpc`.
    pub(crate) fn rpc(&self, responses: &Value, extra: &[&str]) -> Rpc {
        let mut args = vec!["--mode", "rpc"];
        args.extend_from_slice(extra);
        let mut command = self.command(responses, &args);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line}"));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Rpc {
            child,
            stdin: Some(stdin),
            lines: rx,
            seen: Vec::new(),
            next_id: 0,
        }
    }
}

/// A running RPC child.
pub(crate) struct Rpc {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    /// Every record received so far, in order.
    pub(crate) seen: Vec<Value>,
    next_id: u64,
}

impl Rpc {
    /// Writes one raw line.
    pub(crate) fn write_raw(&mut self, raw: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(raw.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Sends a command with a fresh id; returns the id.
    pub(crate) fn send(&mut self, mut command: Value) -> String {
        self.next_id += 1;
        let id = format!("t{}", self.next_id);
        command["id"] = json!(id);
        self.write_raw(&format!("{command}\n"));
        id
    }

    /// Next record (panics after [`TIMEOUT`]).
    pub(crate) fn next(&mut self) -> Value {
        match self.lines.recv_timeout(TIMEOUT) {
            Ok(value) => {
                self.seen.push(value.clone());
                value
            }
            Err(RecvTimeoutError::Timeout) => panic!("timed out; seen: {:#?}", self.seen),
            Err(RecvTimeoutError::Disconnected) => panic!("stdout closed; seen: {:#?}", self.seen),
        }
    }

    /// Next record within `wait`, if any.
    pub(crate) fn try_next(&mut self, wait: Duration) -> Option<Value> {
        let value = self.lines.recv_timeout(wait).ok()?;
        self.seen.push(value.clone());
        Some(value)
    }

    /// Reads until `matches`; returns the match and everything before it.
    pub(crate) fn until(&mut self, matches: impl Fn(&Value) -> bool) -> (Value, Vec<Value>) {
        let mut before = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out; seen: {:#?}",
                self.seen
            );
            let value = self.next();
            if matches(&value) {
                return (value, before);
            }
            before.push(value);
        }
    }

    /// Reads until the response to `id`.
    pub(crate) fn response(&mut self, id: &str) -> Value {
        self.until(|v| v["type"] == "response" && v["id"] == id).0
    }

    /// Sends `command` and returns its response.
    pub(crate) fn call(&mut self, command: Value) -> Value {
        let id = self.send(command);
        self.response(&id)
    }

    /// `data` of a successful response.
    pub(crate) fn ok(&mut self, command: Value) -> Value {
        let response = self.call(command);
        assert_eq!(response["success"], true, "{response:#}");
        response.get("data").cloned().unwrap_or(Value::Null)
    }

    /// `data` of a successful response and the records written before it (pi writes the
    /// events a command causes before its reply).
    pub(crate) fn ok_events(&mut self, command: Value) -> (Value, Vec<Value>) {
        let id = self.send(command);
        let (response, before) = self.until(|v| v["type"] == "response" && v["id"] == id);
        assert_eq!(response["success"], true, "{response:#}");
        (response.get("data").cloned().unwrap_or(Value::Null), before)
    }

    /// `error` of a failed response.
    pub(crate) fn err(&mut self, command: Value) -> String {
        let response = self.call(command);
        assert_eq!(response["success"], false, "{response:#}");
        response["error"].as_str().unwrap().to_owned()
    }

    /// Reads until both the response to `id` and a record matching `matches` arrived (in
    /// either order: replies and events travel separately).
    pub(crate) fn response_and(
        &mut self,
        id: &str,
        matches: impl Fn(&Value) -> bool,
    ) -> (Value, Value) {
        let (mut reply, mut found) = (None, None);
        while reply.is_none() || found.is_none() {
            let value = self.next();
            if value["type"] == "response" && value["id"] == id {
                reply = Some(value);
            } else if found.is_none() && matches(&value) {
                found = Some(value);
            }
        }
        (reply.unwrap(), found.unwrap())
    }

    /// Reads until an event of `kind`.
    pub(crate) fn event(&mut self, kind: &str) -> Value {
        self.until(|v| v["type"] == kind).0
    }

    /// Prompts and waits for `agent_settled`; returns the events of the run.
    pub(crate) fn prompt_and_settle(&mut self, message: &str) -> Vec<Value> {
        let id = self.send(json!({"type": "prompt", "message": message}));
        let response = self.response(&id);
        assert_eq!(response["data"]["disposition"], "started", "{response:#}");
        let (settled, mut events) = self.until(|v| v["type"] == "agent_settled");
        events.push(settled);
        events
    }

    /// Closes stdin and waits for the exit code.
    pub(crate) fn close(mut self) -> Option<i32> {
        drop(self.stdin.take());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            assert!(Instant::now() < deadline, "oino did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Event `type`s of `events`, without `message_update`s.
pub(crate) fn kinds(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["type"] != "message_update")
        .map(|e| e["type"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Session file text lines as JSON.
pub(crate) fn jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
