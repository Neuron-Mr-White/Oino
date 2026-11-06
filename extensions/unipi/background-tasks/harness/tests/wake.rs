//! `unipi.background-tasks` as a real component in the wasm host: a finished task must
//! deliver its `<background-task-notification>` and wake the agent (G-04) — including
//! after the host restarted, and when another load of the extension replaced the
//! process-global registration and died.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, ComponentSource, EventKind, ExtensionId, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const CRATE: &str = "unipi-background-tasks";
const DIR: &str = "background-tasks";
const GUEST: &str = "unipi_background_tasks";
const EXTENSION_ID: &str = "unipi.background-tasks";
const NOTIFICATION_TYPE: &str = "background-task-notification";
const BG_TASKS_ENTRY: &str = "unipi:bg-tasks";

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

/// Builds the component once (in `EXT_WORKSPACE` or this checkout's `extensions/`).
fn wasm_dir() -> &'static PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let ws = std::env::var_os("EXT_WORKSPACE").map_or_else(extensions_dir, PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws)
            .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"])
            .args(["-p", CRATE]);
        for var in [
            "CARGO_TARGET_DIR",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_MANIFEST_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        extensions_dir().join("target/wasm32-wasip2/release")
    })
}

struct Env {
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
    agent: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        // Not under TMPDIR: the extension treats the temp dir as task-output territory.
        let base = extensions_dir().join("target/bg-harness");
        std::fs::create_dir_all(&base).unwrap();
        let project = tempfile::Builder::new()
            .prefix("bgproj")
            .tempdir_in(&base)
            .unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: {
                let host = RecordingHostServices::new();
                for program in ["sh", "bash", "cmd", "powershell.exe"] {
                    host.allow_process_command(program);
                }
                host
            },
            agent,
            project,
        }
    }

    /// SIGKILL of the host process for the guest (core.md §2.1.8): the runtime dies, the
    /// agent dir and the recorded calls survive; a fresh runtime is built over them. The
    /// shared hub (tasks, secrets, sticky bus) is process-wide, exactly as across an app
    /// restart in one host process.
    fn restart(&mut self) {
        let mut options =
            WasmRuntimeOptions::new(HostConfig::new(self.agent.path(), self.project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        self.runtime = WasmRuntime::new(options).unwrap();
    }

    fn cx(&self) -> CallContext {
        CallContext {
            session_id: "s1".into(),
            mode: Mode::Rpc,
            has_ui: true,
            cwd: self.project.path().to_path_buf(),
            kind: CallKind::Tool,
            ..CallContext::default()
        }
    }

    /// Loads the component the way production does (`CoreLoader`): the host is wrapped by
    /// the shared hub first, so the extension's wake route goes through the same
    /// composite the notification path captures (G-04).
    async fn load(&self) -> Arc<dyn ExtensionInstance> {
        self.load_with_host(Arc::clone(&self.host) as Arc<dyn HostServices>)
            .await
    }

    async fn load_with_host(&self, host: Arc<dyn HostServices>) -> Arc<dyn ExtensionInstance> {
        let id = ExtensionId::new(EXTENSION_ID).unwrap();
        let hub = self
            .runtime
            .local_services()
            .expect("local services")
            .clone();
        let wrapped: Arc<dyn HostServices> = hub.wrap_for_test(&id, host);
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{DIR}/extension.json")))
                .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm_dir().join(format!("{GUEST}.wasm"))),
            root: None,
            bundled: true,
        };
        let ext = self.runtime.load(package, wrapped).await.unwrap();
        ext.init(InitInput::default(), self.cx()).await.unwrap();
        self.event(
            &ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
        ext
    }

    async fn event(&self, ext: &Arc<dyn ExtensionInstance>, kind: EventKind, payload: Value) {
        ext.on_event(kind, RawJson::from(payload), self.cx())
            .await
            .unwrap();
    }

    /// Runs the `bg_run` tool (notify/trigger default on).
    async fn bg_run(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, command: &str) -> String {
        let mut cx = self.cx();
        cx.tool_call_id = Some("t1".into());
        let inv = oino_extension_api::ToolInvocation {
            tool_call_id: "t1".into(),
            name: "bg_run".into(),
            args: RawJson::from(json!({"name": name, "command": command, "isAgent": false})),
            parent_tool_call_id: None,
        };
        let result = ext
            .run_tool(inv, oino_extension_api::CancellationToken::new(), cx)
            .await
            .unwrap()
            .to_value();
        result["details"]["task"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// `sendMessage` details recorded on this host.
    fn sent_messages(&self) -> Vec<Value> {
        self.host
            .calls_named("session_write")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["sendMessage"].is_object())
            .collect()
    }

    /// Waits until `f` holds (polling the host's recorded calls), or panics.
    async fn wait_for(&self, what: &str, f: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if f(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "timed out waiting for {what}; session_write calls: {:?}",
            self.host.calls_named("session_write")
        );
    }
}

fn wake_notifications(env: &Env) -> Vec<Value> {
    env.sent_messages()
        .into_iter()
        .filter(|d| d["sendMessage"]["customType"] == NOTIFICATION_TYPE)
        .collect()
}

fn notified_entries(env: &Env) -> Vec<Value> {
    env.host
        .calls_named("session_write")
        .into_iter()
        .map(|c| c.detail)
        .filter(|d| d["appendEntry"] == BG_TASKS_ENTRY)
        .filter_map(|d| d["data"]["local"].as_object().map(|_| d["data"].clone()))
        .collect()
}

/// The happy path: a finished task delivers `<background-task-notification>` as a
/// follow-up that triggers a turn, and the durable local snapshot records `notified`.
#[tokio::test(flavor = "multi_thread")]
async fn bg_run_completion_notifies_and_wakes() {
    let env = Env::new();
    let ext = env.load().await;
    let id = env.bg_run(&ext, "Echo harness", "echo harness-ok").await;
    assert!(!id.is_empty(), "bg_run returned a task id");
    env.wait_for("the completion wake", |env| {
        !wake_notifications(env).is_empty()
    })
    .await;
    let wake = wake_notifications(&env).pop().unwrap();
    assert_eq!(
        wake["sendMessage"]["details"]["taskId"],
        json!(id),
        "the notification names the task"
    );
    assert!(
        wake["sendMessage"]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("harness"),
        "the extension rendered the notification text: {wake}"
    );
    assert_eq!(
        wake["options"]["triggerTurn"],
        json!(true),
        "the agent is woken: {wake}"
    );
    env.wait_for("the notified local snapshot", |env| {
        notified_entries(env)
            .iter()
            .any(|d| d["local"][&id]["notified"] == json!(true))
    })
    .await;
}

/// G-04, the regression: a SECOND load of the extension (another services set) replaced
/// the process-global registration and then died. A task started by the first load must
/// still deliver its notification and wake — through the route captured at start, and
/// rendered by any live registration.
#[tokio::test(flavor = "multi_thread")]
async fn completion_survives_a_newer_load_of_the_extension_dying() {
    let env = Env::new();
    let ext = env.load().await;
    let id = env
        .bg_run(&ext, "Slow echo", "sleep 2 && echo slow-ok")
        .await;

    // A second services set loads the extension (the hub entry is replaced, last one
    // wins) and is disposed before the task ends: its instance and host drop.
    {
        let mut options =
            WasmRuntimeOptions::new(HostConfig::new(env.agent.path(), env.project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        let runtime2 = WasmRuntime::new(options).unwrap();
        let host2 = RecordingHostServices::new();
        let id2 = ExtensionId::new(EXTENSION_ID).unwrap();
        let hub = runtime2.local_services().expect("local services").clone();
        let wrapped: Arc<dyn HostServices> =
            hub.wrap_for_test(&id2, Arc::clone(&host2) as Arc<dyn HostServices>);
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{DIR}/extension.json")))
                .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm_dir().join(format!("{GUEST}.wasm"))),
            root: None,
            bundled: true,
        };
        let ext2 = runtime2.load(package, wrapped).await.unwrap();
        ext2.init(InitInput::default(), env.cx()).await.unwrap();
        drop(ext2);
        drop(host2);
        drop(runtime2);
    }

    env.wait_for(
        "the completion wake through the older registration",
        |env| !wake_notifications(env).is_empty(),
    )
    .await;
    let wake = wake_notifications(&env).pop().unwrap();
    assert_eq!(wake["sendMessage"]["details"]["taskId"], json!(id));
    assert_eq!(wake["options"]["triggerTurn"], json!(true));
    env.wait_for("the notified local snapshot", |env| {
        notified_entries(env)
            .iter()
            .any(|d| d["local"][&id]["notified"] == json!(true))
    })
    .await;
}

/// G-04, the reported repro: bg_run, restart the host, bg_run again — both completions
/// reach the chat and the agent is woken both times.
#[tokio::test(flavor = "multi_thread")]
async fn bg_run_before_and_after_a_host_restart_both_wake() {
    let mut env = Env::new();
    {
        let ext = env.load().await;
        let first = env.bg_run(&ext, "Echo one", "echo one-ok").await;
        env.wait_for("the first completion wake", |env| {
            wake_notifications(env).len() == 1
        })
        .await;
        assert_eq!(
            wake_notifications(&env)[0]["sendMessage"]["details"]["taskId"],
            json!(first)
        );
    }
    env.restart();
    {
        let ext = env.load().await;
        let second = env.bg_run(&ext, "Echo two", "echo two-ok").await;
        env.wait_for("the second completion wake", |env| {
            wake_notifications(env).len() == 2
        })
        .await;
        assert_eq!(
            wake_notifications(&env)[1]["sendMessage"]["details"]["taskId"],
            json!(second)
        );
        assert_eq!(
            wake_notifications(&env)[1]["options"]["triggerTurn"],
            json!(true)
        );
    }
}

/// v3 Execute service uses the same registry, bounded task output and automatic delivery.
#[tokio::test(flavor = "multi_thread")]
async fn execute_task_output_peek_block_stop_and_mixed_ids() {
    let env = Env::new();
    let ext = env.load().await;
    let launched = ext.handle_service("tasks.shell", RawJson::from(json!({
        "command":"sleep 1; echo v3-completed", "summary":"Run v3 background acceptance", "riskLevel":"low", "timeout":60, "shell":"bash"
    })), &ExtensionId::new("oino.frontend").unwrap(), env.cx()).await.unwrap().to_value();
    let id = launched["details"]["task_id"].as_str().unwrap();
    assert!(std::path::Path::new(launched["details"]["log_path"].as_str().unwrap()).exists());
    async fn tool(env: &Env, ext: &Arc<dyn ExtensionInstance>, name: &str, args: Value) -> Value {
        ext.run_tool(
            oino_extension_api::ToolInvocation {
                tool_call_id: "v3-read".into(),
                name: name.into(),
                args: RawJson::from(args),
                parent_tool_call_id: None,
            },
            oino_extension_api::CancellationToken::new(),
            env.cx(),
        )
        .await
        .unwrap()
        .to_value()
    }
    let peek = tool(
        &env,
        &ext,
        "task_output",
        json!({"task_id":id,"block":false}),
    )
    .await;
    assert_eq!(peek["details"]["state"], "running");
    let blocked = tool(
        &env,
        &ext,
        "task_output",
        json!({"task_id":id,"block":true,"timeout":5000}),
    )
    .await;
    assert_eq!(blocked["details"]["state"], "done");
    assert!(
        blocked["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("v3-completed")
    );
    env.wait_for("v3 automatic completion", |env| {
        !wake_notifications(env).is_empty()
    })
    .await;
    let wake = wake_notifications(&env).pop().unwrap();
    assert!(
        wake["sendMessage"]["content"]
            .as_str()
            .unwrap()
            .contains("v3-completed")
    );
    assert_eq!(wake["options"]["triggerTurn"], true);
    let hub = env.runtime.local_services().unwrap();
    let tasks = hub.tasks();
    let owner = ExtensionId::new("unipi.subagents").unwrap();
    let spec = serde_json::from_value(json!({"name":"Mixed id subagent", "command":"Inspect", "isAgent":true, "externalId":"agent-acceptance-42", "externalState":"done", "externalOutput":"subagent-result", "notifyOnCompletion":false})).unwrap();
    tasks
        .start(&owner, "s1", env.project.path(), spec, Arc::new(|_| {}))
        .unwrap();
    let sub = tool(
        &env,
        &ext,
        "task_output",
        json!({"task_id":"agent-acceptance-42","block":false}),
    )
    .await;
    assert!(
        sub["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("subagent-result")
    );
    let running = ext.handle_service("tasks.shell",RawJson::from(json!({"command":"trap '' TERM; sleep 30 & wait","summary":"Stop v3 background acceptance","riskLevel":"low","shell":"bash"})),&ExtensionId::new("oino.frontend").unwrap(),env.cx()).await.unwrap().to_value();
    let stop_id = running["details"]["task_id"].as_str().unwrap();
    let stopped = tool(&env, &ext, "task_stop", json!({"task_id":stop_id})).await;
    assert_eq!(stopped["details"]["state"], "killed");
}

/// Manifest process capability must never substitute for host execution consent.
#[tokio::test(flavor = "multi_thread")]
async fn task_service_policy_denial_precedes_all_side_effects() {
    use oino_extension_api::HostError;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Deny(Mutex<Vec<(CallContext, String, Vec<String>)>>);
    impl HostServices for Deny {
        fn check_process(
            &self,
            _ext: &ExtensionId,
            cx: &CallContext,
            program: &str,
            args: &[String],
        ) -> Result<(), HostError> {
            self.0
                .lock()
                .unwrap()
                .push((cx.clone(), program.into(), args.to_vec()));
            Err(HostError::Failed("needs Autonomy High".into()))
        }
    }
    let env = Env::new();
    let policy = Arc::new(Deny::default());
    let ext = env.load_with_host(policy.clone()).await;
    for child in [false, true] {
        let mut cx = env.cx();
        cx.kind = CallKind::Service;
        cx.mode = Mode::Print;
        cx.has_ui = false;
        cx.is_child = child;
        cx.child_role = child.then(|| "subagent".into());
        let result = ext.handle_service("tasks.shell", RawJson::from(json!({
            "command":"printf bypass-ok > marker", "summary":"Audit policy bypass", "riskLevel":"high", "shell":"bash"
        })), &ExtensionId::new("oino.frontend").unwrap(), cx.clone()).await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("needs Autonomy High")
        );
        let calls = policy.0.lock().unwrap();
        let (seen, program, args) = calls.last().unwrap();
        assert_eq!(seen, &cx);
        assert_eq!(program, "bash");
        assert_eq!(args, &["-c", "printf bypass-ok > marker"]);
        assert!(!env.project.path().join("marker").exists());
        assert!(
            !env.agent.path().join("tmp/tasks").exists(),
            "denial must not create an output path"
        );
        assert!(
            env.runtime
                .local_services()
                .unwrap()
                .tasks()
                .list("s1")
                .is_empty()
        );
    }
}
