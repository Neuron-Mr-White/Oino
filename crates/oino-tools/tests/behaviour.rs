//! Behaviour beyond the pi goldens: streaming updates, detach, D12 environment, the `!cmd`
//! executor, process-tree kill, temp files and the mutation queue.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oino_agent::{AgentTool, AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{JsonObject, UserContent};
use oino_tools::{
    BuiltinToolName, DEFAULT_ACTIVE_TOOLS, LocalEnv, SessionEnv, ShellRunOptions, ToolConfig,
    ToolContext, builtin_tools, run_shell_command, tool_by_name,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn temp_root(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("behaviour")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir.join("work")).unwrap();
    dir
}

fn config(root: &std::path::Path) -> ToolConfig {
    ToolConfig::new(
        root.join("work").to_string_lossy().into_owned(),
        root.join("agent").to_string_lossy().into_owned(),
    )
}

fn args(value: serde_json::Value) -> JsonObject {
    match value {
        serde_json::Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

fn call(id: &str, value: serde_json::Value) -> ToolInvocation {
    ToolInvocation {
        tool_call_id: id.to_owned(),
        args: args(value),
        parent_tool_call_id: None,
    }
}

fn text(result: &AgentToolResult) -> &str {
    match &result.content[0] {
        UserContent::Text(text) => &text.text,
        UserContent::Image(_) => panic!("image"),
    }
}

/// Runs a call through the agent loop's `run_tool_call` (the only public way to observe
/// updates) and returns the result plus every partial update.
async fn run_with_updates(
    tool: Arc<dyn AgentTool>,
    invocation: ToolInvocation,
) -> (AgentToolResult, Vec<AgentToolResult>) {
    let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&updates);
    let callback = move |update: AgentToolResult| -> futures::future::BoxFuture<'_, ()> {
        sink.lock().unwrap().push(update);
        Box::pin(async {})
    };
    let assistant: oino_ai::AssistantMessage = serde_json::from_value(json!({
        "content": [], "api": "faux", "provider": "faux", "model": "faux", "stopReason": "toolUse"
    }))
    .unwrap();
    let context = oino_agent::AgentContext {
        messages: Vec::new(),
        tools: vec![Arc::clone(&tool)],
    };
    let tool_call = oino_ai::ToolCall {
        id: invocation.tool_call_id,
        name: tool.meta().name.clone(),
        arguments: invocation.args,
        ..oino_ai::ToolCall::default()
    };
    let outcome = oino_agent::run_tool_call(
        tool_call,
        oino_agent::RunToolCallOptions {
            assistant_message: &assistant,
            context: &context,
            tools: &context.tools,
            before_tool_call: None,
            after_tool_call: None,
            cancel: CancellationToken::new(),
            parent_tool_call_id: None,
            on_update: Some(&callback),
        },
    )
    .await;
    let updates = updates.lock().unwrap().clone();
    (outcome.result, updates)
}

#[test]
fn registry_matches_pi() {
    let root = temp_root("registry");
    let names: Vec<String> = builtin_tools(&config(&root))
        .iter()
        .map(|tool| tool.meta().name.clone())
        .collect();
    let expected: Vec<&str> = if cfg!(windows) {
        vec![
            "read",
            "bash",
            "powershell",
            "edit",
            "write",
            "grep",
            "find",
            "ls",
        ]
    } else {
        vec!["read", "bash", "edit", "write", "grep", "find", "ls"]
    };
    assert_eq!(names, expected);
    assert_eq!(
        DEFAULT_ACTIVE_TOOLS.map(BuiltinToolName::as_str),
        ["read", "bash", "edit", "write"]
    );
    assert_eq!(
        tool_by_name("powershell", config(&root)).is_some(),
        cfg!(windows)
    );
}

#[tokio::test]
async fn bash_streams_throttled_updates() {
    let root = temp_root("updates");
    let tool = tool_by_name("bash", config(&root)).unwrap();
    let (result, updates) = run_with_updates(
        tool,
        call(
            "u1",
            json!({"command": "for i in 1 2 3 4 5; do echo $i; sleep 0.05; done"}),
        ),
    )
    .await;
    assert_eq!(text(&result), "1\n2\n3\n4\n5\n");
    assert_eq!(
        updates[0],
        AgentToolResult::default(),
        "pi's initial empty update"
    );
    assert!(updates.len() >= 3, "throttled snapshots: {}", updates.len());
    let last = updates.last().unwrap();
    assert_eq!(text(last), "1\n2\n3\n4\n5\n");
    assert_eq!(last.details, Some(json!({})));
}

#[tokio::test]
async fn bash_detach_hands_over_the_process() {
    let root = temp_root("detach");
    let config = config(&root);
    let registry = config.bash_detach.clone();
    let tool = tool_by_name("bash", config).unwrap();
    let running = tokio::spawn(tool.execute(
        call("d1", json!({"command": "echo first; sleep 1; echo second"})),
        UpdateSink::discard(),
        CancellationToken::new(),
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(registry.is_running("d1"));
    let mut detached = registry
        .detach("d1", "task-7", "user request")
        .await
        .unwrap();
    let result = running.await.unwrap().unwrap();
    assert_eq!(
        text(&result),
        "first\n\n\n[Detached to background task task-7: user request]"
    );
    assert!(!result.is_error);
    assert_eq!(detached.output_so_far, "first\n");
    let mut rest = Vec::new();
    while let Some(chunk) = detached.process.output.recv().await {
        rest.extend(chunk);
    }
    assert_eq!(String::from_utf8(rest).unwrap(), "second\n");
    let status = detached.process.control.wait().await.unwrap();
    assert_eq!(status.shell_exit_code(), 0);
    assert!(!registry.is_running("d1"));
    assert!(registry.detach("d1", "x", "y").await.is_none());
}

struct Session;

impl ToolContext for Session {
    fn session_env(&self) -> Option<SessionEnv> {
        Some(SessionEnv {
            session_id: "s-1".into(),
            session_file: None,
            provider: Some("p".into()),
            model: Some("m".into()),
            reasoning_level: Some("low".into()),
        })
    }
}

#[tokio::test]
async fn bash_exports_oino_and_pi_session_variables() {
    let root = temp_root("env");
    let mut config = config(&root);
    config.context = Arc::new(Session);
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.push(("OINO_SESSION_FILE".into(), "inherited".into()));
    config.process_env = Some(env);
    let tool = tool_by_name("bash", config).unwrap();
    let result = tool
        .execute(
            call("e1", json!({"command": "echo \"$OINO_SESSION_ID $PI_SESSION_ID $OINO_MODEL $OINO_REASONING_LEVEL [$OINO_SESSION_FILE] $PATH\""})),
            UpdateSink::discard(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let out = text(&result);
    assert!(out.starts_with("s-1 s-1 m low [] "), "{out}");
    let bin = root.join("agent").join("bin");
    assert!(out.contains(&format!("{}:", bin.display())), "{out}");
}

#[tokio::test]
async fn bash_truncation_writes_private_temp_file() {
    let root = temp_root("tempfile");
    let tool = tool_by_name("bash", config(&root)).unwrap();
    let result = tool
        .execute(
            call("t1", json!({"command": "seq 1 5000"})),
            UpdateSink::discard(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let path = result.details.as_ref().unwrap()["fullOutputPath"]
        .as_str()
        .unwrap()
        .to_owned();
    let tmp = root.join("agent").join("tmp");
    assert!(path.starts_with(&*tmp.to_string_lossy()), "{path}");
    let name = std::path::Path::new(&path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("oino-bash-")
            && std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext == "log")
            && name.len() == "oino-bash-".len() + 16 + 4
    );
    let full = std::fs::read_to_string(&path).unwrap();
    assert_eq!(full.lines().count(), 5000);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(text(&result).ends_with(&format!(
        "[Showing lines 3001-5000 of 5000. Full output: {path}]"
    )));
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(unix)]
#[tokio::test]
async fn abort_kills_the_process_tree() {
    let root = temp_root("killtree");
    let pid_file = root.join("child.pid");
    let tool = tool_by_name("bash", config(&root)).unwrap();
    let cancel = CancellationToken::new();
    let command = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
    let running = tokio::spawn(tool.execute(
        call("k1", json!({"command": command})),
        UpdateSink::discard(),
        cancel.clone(),
    ));
    let mut pid = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(parsed) = std::fs::read_to_string(&pid_file)
            .map_err(|_| ())
            .and_then(|text| text.trim().parse::<i32>().map_err(|_| ()))
        {
            pid = Some(parsed);
            break;
        }
    }
    let pid = pid.unwrap();
    assert!(alive(pid));
    cancel.cancel();
    let error = running.await.unwrap().unwrap_err();
    assert_eq!(error.0, "Command aborted");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!alive(pid), "grandchild survived");
}

#[tokio::test]
async fn executor_sanitises_and_reports_exit_code() {
    let root = temp_root("executor");
    let options = ShellRunOptions {
        agent_dir: root.join("agent").to_string_lossy().into_owned(),
        ..ShellRunOptions::default()
    };
    let mut chunks = String::new();
    let mut on_chunk = |chunk: &str| chunks.push_str(chunk);
    let outcome = run_shell_command(
        &LocalEnv::new(),
        "printf '\\033[31mred\\033[0m\\r\\n\\001ok\\n'; exit 4",
        &root.join("work").to_string_lossy(),
        &options,
        &mut on_chunk,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.output, "red\nok\n");
    assert_eq!(outcome.exit_code, Some(4));
    assert!(!outcome.cancelled && !outcome.truncated);
    assert_eq!(chunks, "red\nok\n");
}

#[tokio::test]
async fn executor_cancel_returns_output_so_far() {
    let root = temp_root("executor-cancel");
    let options = ShellRunOptions {
        agent_dir: root.join("agent").to_string_lossy().into_owned(),
        ..ShellRunOptions::default()
    };
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let mut on_chunk = |_: &str| {};
    let outcome = run_shell_command(
        &LocalEnv::new(),
        "echo before; sleep 10",
        &root.join("work").to_string_lossy(),
        &options,
        &mut on_chunk,
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(outcome.output, "before\n");
    assert_eq!(outcome.exit_code, None);
    assert!(outcome.cancelled);
}

#[tokio::test]
async fn executor_missing_cwd_is_an_error() {
    let root = temp_root("executor-cwd");
    let mut on_chunk = |_: &str| {};
    let error = run_shell_command(
        &LocalEnv::new(),
        "true",
        &root.join("missing").to_string_lossy(),
        &ShellRunOptions::default(),
        &mut on_chunk,
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(error.0.starts_with("Working directory does not exist: "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_edits_of_one_file_are_serialised() {
    let root = temp_root("queue");
    let config = config(&root);
    let file = root.join("work").join("q.txt");
    let lines: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
    std::fs::write(&file, format!("{}\n", lines.join("\n"))).unwrap();
    let tool = tool_by_name("edit", config).unwrap();
    let mut handles = Vec::new();
    for i in 0..20 {
        let future = tool.execute(
            call(
                &format!("q{i}"),
                json!({"path": "q.txt", "edits": [{"oldText": format!("line {i}\n"), "newText": format!("LINE {i}\n")}]}),
            ),
            UpdateSink::discard(),
            CancellationToken::new(),
        );
        handles.push(tokio::spawn(future));
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.matches("LINE").count(), 20, "{content}");
}
