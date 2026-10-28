//! Print mode (`-p`), JSON mode, metadata commands and CLI errors of the `oino` binary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use serde_json::{Value, json};
use support::Sandbox;

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn print_mode_prints_last_assistant_text_blocks() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(
        &json!([{"text": "hello"}]),
        &["--no-session", "-p", "hi"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hello\n");
}

#[test]
fn print_mode_runs_every_message_and_reads_files_and_stdin() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.cwd().join("notes.md"), "file body").unwrap();
    let output = sandbox.run(
        &json!([{"text": "first"}, {"text": "second"}]),
        &["--no-session", "-p", "@notes.md", "one", "two"],
        Some("piped\n"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "second\n");

    // The session file records exactly what was sent.
    let sessions = sandbox.dir.path().join("sessions");
    let dir = sessions.display().to_string();
    let output = sandbox.run(
        &json!([{"text": "a"}, {"text": "b"}]),
        &["--session-dir", &dir, "-p", "@notes.md", "one", "two"],
        Some("piped\n"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let file = walk(&sessions).into_iter().next().unwrap();
    let users: Vec<String> = support::jsonl(&file)
        .into_iter()
        .filter(|e| e["type"] == "message" && e["message"]["role"] == "user")
        .map(|e| {
            e["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let notes = sandbox.cwd().join("notes.md").display().to_string();
    assert_eq!(
        users,
        [
            format!("piped<file name=\"{notes}\">\nfile body\n</file>\none"),
            "two".to_owned()
        ]
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            files.push(path);
        }
    }
    files
}

/// pi `--session-id`: an unknown id creates the session with that header id, an existing one
/// opens it (known issue 14).
#[test]
fn session_id_creates_the_session_like_pi() {
    let sandbox = Sandbox::new();
    let sessions = sandbox.dir.path().join("sessions");
    let dir = sessions.display().to_string();
    let output = sandbox.run(
        &json!([{"text": "hello"}]),
        &[
            "--session-dir",
            &dir,
            "--session-id",
            "run-abc123",
            "-p",
            "hi",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hello\n");
    let files = walk(&sessions);
    assert_eq!(files.len(), 1, "{files:?}");
    let entries = support::jsonl(&files[0]);
    assert_eq!(entries[0]["type"], json!("session"));
    assert_eq!(entries[0]["id"], json!("run-abc123"));
    assert_eq!(
        entries[0]["cwd"],
        json!(sandbox.cwd().display().to_string())
    );

    // Reopening by id appends to the same file.
    let output = sandbox.run(
        &json!([{"text": "again"}]),
        &[
            "--session-dir",
            &dir,
            "--session-id",
            "run-abc123",
            "-p",
            "second",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let reopened = walk(&sessions);
    assert_eq!(reopened, files, "no second file for a known id");
    let entries = support::jsonl(&files[0]);
    let users: Vec<&Value> = entries
        .iter()
        .filter(|e| e["type"] == json!("message") && e["message"]["role"] == json!("user"))
        .collect();
    assert_eq!(users.len(), 2, "{}", entries.len());

    // Ids pi rejects stay rejected.
    let rejected = sandbox.run(
        &json!([{"text": "x"}]),
        &["--session-dir", &dir, "--session-id", "bad/id", "-p", "hi"],
        None,
    );
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        stderr(&rejected).contains("Session id must be"),
        "{}",
        stderr(&rejected)
    );
}

#[test]
fn print_mode_error_exit_code() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(
        &json!([{"error": "provider exploded"}]),
        &["--no-session", "-p", "hi"],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("provider exploded"),
        "{}",
        stderr(&output)
    );

    // `length` exits 1 only when the truncated reply stays last; with pi 1.4 length
    // recovery (core) the reply is omitted, so that rule is covered by the unit tests.
}

#[test]
fn json_mode_streams_header_then_events() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(
        &json!([{"text": "hello"}]),
        &["--no-session", "--mode", "json", "hi"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let lines: Vec<Value> = stdout(&output)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["type"], "session");
    assert_eq!(lines[0]["version"], 3);
    assert_eq!(lines[1]["type"], "agent_start");
    assert_eq!(lines.last().unwrap()["type"], "agent_settled");
    for line in &lines {
        assert!(
            line["type"] != "extension_ui_request" && line["type"] != "response",
            "{line}"
        );
        if line["type"] == "message_update" {
            assert!(line.get("message").is_none());
            assert!(line.get("usage").is_some());
            assert!(line["assistantMessageEvent"].get("partial").is_none());
        }
    }
    let deltas: String = lines
        .iter()
        .filter(|l| l["assistantMessageEvent"]["type"] == "text_delta")
        .map(|l| {
            l["assistantMessageEvent"]["delta"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(deltas, "hello");
}

#[test]
fn metadata_commands_and_cli_errors() {
    let sandbox = Sandbox::new();
    let version = sandbox.run(&json!([]), &["--version"], None);
    assert_eq!(stdout(&version), format!("{}\n", env!("CARGO_PKG_VERSION")));

    let help = sandbox.run(&json!([]), &["--help"], None);
    assert_eq!(help.status.code(), Some(0));
    assert!(stdout(&help).starts_with("oino - AI coding assistant"));
    assert!(stdout(&help).contains("--no-builtin-tools, -nbt"));

    let list = sandbox.run(&json!([]), &["--list-models"], None);
    assert_eq!(list.status.code(), Some(0), "{}", stderr(&list));
    assert_eq!(
        stdout(&list),
        "provider  model   context  max-out  thinking  images\nfaux      faux-1  128K     16.4K    yes       yes   \n"
    );
    let none = sandbox.run(&json!([]), &["--list-models", "zzz"], None);
    assert_eq!(stdout(&none), "No models matching \"zzz\"\n");

    let subcommand = std::process::Command::new(support::oino_bin())
        .args(["serve"])
        .env("HOME", sandbox.home())
        .output()
        .unwrap();
    assert_eq!(subcommand.status.code(), Some(1));
    assert_eq!(
        stderr(&subcommand).trim(),
        "Error: \"oino serve\" is not available in this build yet"
    );

    let unknown = sandbox.run(&json!([]), &["-z"], None);
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(stderr(&unknown).trim(), "Error: Unknown option: -z");

    let flag = sandbox.run(&json!([]), &["--no-session", "--plan", "-p", "x"], None);
    assert_eq!(flag.status.code(), Some(1));
    assert_eq!(stderr(&flag).trim(), "Error: Unknown option: --plan");

    let fork = sandbox.run(&json!([]), &["--fork", "abc", "--no-session", "-c"], None);
    assert_eq!(
        stderr(&fork).trim(),
        "Error: --fork cannot be combined with --continue, --no-session"
    );

    let missing = sandbox.run(&json!([]), &["--no-session", "-p", "@missing.md"], None);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).starts_with("Error: File not found: "));

    let bad_extension = sandbox.run(&json!([]), &["-e", "builtin:nope", "-p", "x"], None);
    assert_eq!(bad_extension.status.code(), Some(1));
    assert_eq!(
        stderr(&bad_extension).trim(),
        "Error: Failed to load extension \"builtin:nope\": unknown built-in extension \"nope\"\nHint: Start without extensions using \"oino -ne\"."
    );

    let provider = sandbox.run(&json!([]), &["--provider", "faux", "-p", "x"], None);
    assert!(
        stderr(&provider).contains(
            "Error: --provider requires --model (for example: --provider faux --model <pattern>)"
        ),
        "{}",
        stderr(&provider)
    );

    let mut no_key = sandbox.command(&json!([]), &["--no-session", "-p", "x"]);
    no_key.env_remove("OINO_FAUX_API_KEY");
    let no_key = no_key.output().unwrap();
    assert_eq!(no_key.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&no_key.stderr).contains("No models available."),
        "{}",
        String::from_utf8_lossy(&no_key.stderr)
    );
}

#[test]
fn model_flags_select_and_validate() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(
        &json!([{"text": "ok"}]),
        &[
            "--no-session",
            "--model",
            "faux/faux-1:high",
            "--mode",
            "json",
            "x",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ends: Vec<Value> = stdout(&output)
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|l| l["type"] == "message_end" && l["message"]["role"] == "assistant")
        .collect();
    assert_eq!(ends[0]["message"]["thinkingLevel"], "high");

    let unknown = sandbox.run(
        &json!([]),
        &["--no-session", "--model", "zzz-model", "-p", "x"],
        None,
    );
    assert_eq!(unknown.status.code(), Some(1));
    assert!(
        stderr(&unknown).starts_with("Error: "),
        "{}",
        stderr(&unknown)
    );
}

#[test]
fn export_writes_html_from_a_session_file() {
    let sandbox = Sandbox::new();
    let sessions = sandbox.dir.path().join("sessions");
    let dir = sessions.display().to_string();
    let output = sandbox.run(
        &json!([{"text": "a"}]),
        &["--session-dir", &dir, "-p", "x"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let file = walk(&sessions).into_iter().next().unwrap();
    let html = sandbox.dir.path().join("out.html");
    let output = sandbox.run(
        &json!([]),
        &[
            "--export",
            &file.display().to_string(),
            &html.display().to_string(),
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("Exported to: {}\n", html.display())
    );
    assert!(std::fs::read_to_string(html).unwrap().contains("<html"));
    let missing = sandbox.run(&json!([]), &["--export", "/nope.jsonl"], None);
    assert_eq!(
        stderr(&missing).trim(),
        "Error: File not found: /nope.jsonl"
    );
}

#[test]
fn continue_and_session_flags_reopen_sessions() {
    let sandbox = Sandbox::new();
    let sessions = sandbox.dir.path().join("sessions");
    let dir = sessions.display().to_string();
    let first = sandbox.run(
        &json!([{"text": "a"}]),
        &["--session-dir", &dir, "-p", "x"],
        None,
    );
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let file = walk(&sessions).into_iter().next().unwrap();
    let id = support::jsonl(&file)[0]["id"].as_str().unwrap().to_owned();
    let again = sandbox.run(
        &json!([{"text": "b"}]),
        &["--session-dir", &dir, "-c", "-p", "y"],
        None,
    );
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    assert_eq!(walk(&sessions).len(), 1);
    let users = support::jsonl(&file)
        .iter()
        .filter(|e| e["type"] == "message" && e["message"]["role"] == "user")
        .count();
    assert_eq!(users, 2);
    let by_prefix = sandbox.run(
        &json!([{"text": "c"}]),
        &["--session-dir", &dir, "--session", &id[..8], "-p", "z"],
        None,
    );
    assert_eq!(by_prefix.status.code(), Some(0), "{}", stderr(&by_prefix));
    assert_eq!(walk(&sessions).len(), 1);
    let fork = sandbox.run(
        &json!([{"text": "d"}]),
        &["--session-dir", &dir, "--fork", &id, "-p", "w"],
        None,
    );
    assert_eq!(fork.status.code(), Some(0), "{}", stderr(&fork));
    assert_eq!(walk(&sessions).len(), 2);
    let missing = sandbox.run(
        &json!([]),
        &["--session-dir", &dir, "--session", "ffffffff", "-p", "x"],
        None,
    );
    assert_eq!(
        stderr(&missing).trim(),
        "No session found matching 'ffffffff'"
    );
}
