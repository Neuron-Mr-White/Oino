//! Decision B execution acceptance: one host registry for shells and subagents.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{JsonValue, UserContent};
use oino_extension_api::{ExtensionId, HostError};
use oino_tools::{ToolConfig, ToolContext, tool_by_name};
use oino_wasm_host::{TaskService, TaskSpec};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn text(result: &AgentToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            UserContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn config(root: &std::path::Path) -> ToolConfig {
    ToolConfig::new(root.to_string_lossy(), root.join("agent").to_string_lossy())
}
async fn run(
    config: &ToolConfig,
    args: JsonValue,
) -> Result<AgentToolResult, oino_agent::ToolError> {
    tool_by_name(
        if cfg!(windows) { "powershell" } else { "bash" },
        config.clone(),
    )
    .unwrap()
    .execute(
        ToolInvocation {
            tool_call_id: "exec-test".into(),
            args: args.as_object().unwrap().clone(),
            parent_tool_call_id: None,
        },
        UpdateSink::discard(),
        CancellationToken::new(),
    )
    .await
}
async fn done(tasks: &TaskService, id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.get(id).unwrap().finished_at.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
fn spec(command: &str) -> TaskSpec {
    serde_json::from_value(json!({"name":"Acceptance command", "command":command,
        "shell": if cfg!(windows) { "powershell" } else { "bash" }, "riskLevel":"low", "timeoutSeconds":60})).unwrap()
}

#[tokio::test]
async fn background_registry_and_stop() {
    let root = tempfile::tempdir().unwrap();
    let tasks = Arc::new(TaskService::new(root.path()));
    let owner = ExtensionId::new("unipi.background-tasks").unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let id = tasks
        .start(
            &owner,
            "s1",
            root.path(),
            spec("echo shell-result"),
            Arc::new(move |info| {
                tx.send(info).unwrap();
            }),
        )
        .unwrap();
    let notification = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notification.id, id);
    assert_eq!(notification.exit_code, Some(0));
    assert!(
        tasks
            .logs(&id, 32768, false)
            .unwrap()
            .contains("shell-result")
    );
    assert!(std::path::Path::new(&notification.output_path).is_file());
    let external: TaskSpec = serde_json::from_value(json!({"name":"Subagent acceptance", "command":"Inspect project", "externalId":"agent-legacy-42", "externalState":"running", "isAgent":true})).unwrap();
    let child_owner = ExtensionId::new("unipi.subagents").unwrap();
    let sub = tasks
        .start(&child_owner, "s1", root.path(), external, Arc::new(|_| {}))
        .unwrap();
    assert_eq!(sub, "agent-legacy-42");
    assert_ne!(sub, id);
    assert_eq!(tasks.list("s1").len(), 2);
    let update: TaskSpec = serde_json::from_value(json!({"name":"Subagent acceptance", "command":"Inspect project", "externalId":sub, "externalState":"done", "externalOutput":"Agent final report", "isAgent":true})).unwrap();
    tasks
        .start(&child_owner, "s1", root.path(), update, Arc::new(|_| {}))
        .unwrap();
    assert_eq!(tasks.logs(&sub, 32768, true).unwrap(), "Agent final report");
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "trap '' TERM; sleep 30 & wait"
    };
    let long = tasks
        .start(&owner, "s1", root.path(), spec(command), Arc::new(|_| {}))
        .unwrap();
    tasks.kill(&long).unwrap();
    done(&tasks, &long).await;
    assert_eq!(format!("{:?}", tasks.get(&long).unwrap().state), "Killed");
}

#[tokio::test]
async fn timeout_and_detach() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let shell = oino_tools::info::shell_info(
        if cfg!(windows) {
            oino_tools::info::ShellKind::PowerShell
        } else {
            oino_tools::info::ShellKind::Bash
        },
        true,
    );
    assert_eq!(
        shell.parameters["required"],
        json!(["command", "summary", "riskLevel"])
    );
    assert_eq!(shell.parameters["properties"]["timeout"]["default"], 60);
    assert_eq!(shell.parameters["additionalProperties"], false);
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let error = run(
        &cfg,
        json!({"command":command,"summary":"Wait for timeout","riskLevel":"low","timeout":0.05}),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("timed out after 0.05 seconds"));
    #[cfg(unix)]
    {
        let cfg2 = cfg.clone();
        let task = tokio::spawn(async move {
            run(&cfg2, json!({"command":"echo before; sleep 1; echo after","summary":"Detach running command","riskLevel":"low"})).await
        });
        while !cfg.bash_detach.is_running("exec-test") {
            tokio::task::yield_now().await;
        }
        let detached = cfg
            .bash_detach
            .detach("exec-test", "bg-legacy-1", "acceptance")
            .await
            .unwrap();
        assert!(text(&task.await.unwrap().unwrap()).contains("bg-legacy-1"));
        let mut process = detached.process;
        assert_eq!(process.control.wait().await.unwrap().code, Some(0));
        process.control.close_output();
    }
}

#[tokio::test]
async fn output_head_tail_and_legacy_normalization() {
    let root = tempfile::tempdir().unwrap();
    let command = if cfg!(windows) {
        "Write-Output 'HEAD'; Write-Output ('x' * 100000); Write-Output 'TAIL'"
    } else {
        "printf 'HEAD\n'; printf '%100000s' ' '; printf '\nTAIL\n'"
    };
    let result = run(
        &config(root.path()),
        json!({"command":command,"summary":"Print oversized output","riskLevel":"low"}),
    )
    .await
    .unwrap();
    let output = text(&result);
    assert!(output.len() <= 32 * 1024, "{}", output.len());
    assert!(output.starts_with("HEAD"));
    assert!(output.contains("TAIL"));
    let path = result.details.as_ref().unwrap()["fullOutputPath"]
        .as_str()
        .unwrap();
    assert!(std::fs::metadata(path).unwrap().len() > 100000);
    assert!(
        result.structured_content.as_ref().unwrap()["output"]
            .as_str()
            .unwrap()
            .len()
            <= 32 * 1024
    );
    for (name, args, expected) in [
        ("bash", json!({"command":"echo ok"}), "bash"),
        (
            "bg_run",
            json!({"command":"echo ok","name":"Old task","timeoutSeconds":7}),
            if cfg!(windows) { "powershell" } else { "bash" },
        ),
        (
            "bg_status",
            json!({"taskId":"agent-legacy-42"}),
            "task_output",
        ),
        ("bg_logs", json!({"taskId":"bg-legacy-1"}), "task_output"),
        ("bg_kill", json!({"taskId":"bg-legacy-1"}), "task_stop"),
    ] {
        let source = args.clone();
        let normalized = oino_tools::legacy::normalize_call(
            name,
            args.as_object().unwrap(),
            root.path().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(normalized.name, expected);
        assert_eq!(args, source);
        if expected == "bash" || expected == "powershell" {
            assert_eq!(normalized.args["riskLevel"], "high");
        } else {
            assert_eq!(normalized.args["task_id"], source["taskId"]);
        }
    }
}

struct Bridge {
    tasks: Arc<TaskService>,
    cwd: std::path::PathBuf,
}
impl ToolContext for Bridge {
    fn background_shell(
        &self,
        args: JsonValue,
    ) -> futures::future::BoxFuture<'static, Result<AgentToolResult, oino_agent::ToolError>> {
        let tasks = self.tasks.clone();
        let cwd = self.cwd.clone();
        Box::pin(async move {
            let mut spec = spec(args["command"].as_str().unwrap());
            spec.timeout_seconds = args["timeout"].as_f64();
            spec.arguments = Some(args.clone());
            let id = tasks
                .start(
                    &ExtensionId::new("unipi.background-tasks").unwrap(),
                    "s1",
                    &cwd,
                    spec,
                    Arc::new(|_| {}),
                )
                .map_err(|e: HostError| oino_agent::ToolError::new(e.to_string()))?;
            Ok(AgentToolResult {
                details: Some(json!({"task_id":id,"log_path":tasks.get(&id).unwrap().output_path})),
                ..AgentToolResult::text("Started")
            })
        })
    }
}
#[tokio::test]
async fn fire_and_forget_default_timeout() {
    let root = tempfile::tempdir().unwrap();
    let tasks = Arc::new(TaskService::new(root.path()));
    let mut cfg = config(root.path());
    cfg.context = Arc::new(Bridge {
        tasks: tasks.clone(),
        cwd: root.path().into(),
    });
    let result = run(&cfg, json!({"command":"echo background","summary":"Launch background command","riskLevel":"low","fireAndForget":true})).await.unwrap();
    let id = result.details.unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(tasks.get(&id).unwrap().spec.timeout_seconds, Some(60.0));
    assert_eq!(
        tasks.get(&id).unwrap().spec.arguments.as_ref().unwrap()["riskLevel"],
        "low"
    );
    done(&tasks, &id).await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_job_tree() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let command = "$p = Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 60' -PassThru; $p.Id | Set-Content child.pid; Start-Sleep -Seconds 60";
    let error = run(&cfg, json!({"command":command,"summary":"Timeout Windows child tree","riskLevel":"low","timeout":2})).await.unwrap_err();
    assert!(error.to_string().contains("timed out"));
    let pid = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
    let result = run(&cfg, json!({"command":format!("if (Get-Process -Id {} -ErrorAction SilentlyContinue) {{ exit 1 }}", pid.trim()),"summary":"Verify child was killed","riskLevel":"low"})).await.unwrap();
    assert!(!result.is_error);
}
