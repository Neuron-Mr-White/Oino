//! Host background-task service (C12, extension-api §5.5).

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oino_extension_api::{ExtensionId, HostError};
use serde::{Deserialize, Serialize};

use crate::process::{Input, Output, ProcessService, Spawn};
use crate::state::lock;

/// Default output cap.
pub(crate) const DEFAULT_MAX_OUTPUT: u64 = 20 * 1024 * 1024;
const MIN_MAX_OUTPUT: u64 = 1024;
const KEEP_FINISHED: usize = 30;

/// `TaskSpec` (the host's background tasks).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// Shell selected by the Execute tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// Resolved shell executable, arguments, environment and optional stdin from shell tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ShellExecution>,
    /// Declared risk, carried without enforcing a ceiling here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    /// Exact launch arguments for live inspection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
    /// Existing subagent id registered in this same namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Updated subagent state (running/done/failed/killed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<TaskState>,
    /// Latest complete subagent report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_output: Option<String>,
    /// Launches an agent process.
    #[serde(default)]
    pub is_agent: bool,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Kill after this many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<f64>,
    /// Deliver a notification message on completion.
    #[serde(default = "yes")]
    pub notify_on_completion: bool,
    /// The notification triggers a turn.
    #[serde(default = "yes")]
    pub trigger_on_completion: bool,
    /// Output cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<u64>,
}

fn yes() -> bool {
    true
}

/// Prepared shell invocation, preserving shell settings and session environment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellExecution {
    /// Resolved executable.
    pub program: String,
    /// Executable arguments.
    pub args: Vec<String>,
    /// Complete environment.
    pub vars: Vec<(String, String)>,
    /// Script transported via stdin when required.
    pub stdin: Option<String>,
}

impl TaskSpec {
    /// Exact invocation authorized by the host and then used by the task spawner.
    pub(crate) fn process_invocation(&self) -> Result<(String, Vec<String>), HostError> {
        if let Some(execution) = &self.execution {
            return Ok((execution.program.clone(), execution.args.clone()));
        }
        let (shell, args) = match self.shell.as_deref() {
            Some("bash") => ("bash", vec!["-c".to_owned(), self.command.clone()]),
            Some("powershell") if cfg!(windows) => (
                "powershell.exe",
                vec![
                    "-NoProfile".to_owned(),
                    "-NonInteractive".to_owned(),
                    "-Command".to_owned(),
                    format!(
                        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {}",
                        self.command
                    ),
                ],
            ),
            Some(_) => return Err(HostError::InvalidArgument("Unsupported shell".to_owned())),
            None if cfg!(windows) => ("cmd", vec!["/C".to_owned(), self.command.clone()]),
            None => ("sh", vec!["-c".to_owned(), self.command.clone()]),
        };
        Ok((shell.to_owned(), args))
    }
}

/// Task state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    /// Running.
    Running,
    /// Exited 0.
    Done,
    /// Non-zero exit, timeout or output cap.
    Failed,
    /// Killed by request.
    Killed,
}

/// `TaskInfo`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    /// Task id.
    pub id: String,
    /// Owning extension.
    pub owner: String,
    /// Session.
    pub session_id: String,
    /// Spec.
    #[serde(flatten)]
    pub spec: TaskSpec,
    /// State.
    pub state: TaskState,
    /// Exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Failure reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Output file.
    pub output_path: String,
    /// Start (epoch ms).
    pub started_at: i64,
    /// End (epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// Process id.
    pub pid: u32,
}

/// Called when a task finishes.
pub(crate) type OnFinish = Arc<dyn Fn(TaskInfo) + Send + Sync>;

/// How a running task is killed.
enum Killer {
    /// A process the service spawned.
    Child(Arc<crate::process::ChildHandle>),
    /// A detached `bash` call the service adopted.
    Adopted(tokio_util::sync::CancellationToken),
}

impl Killer {
    fn kill(&self) {
        match self {
            Self::Child(child) => child.stop(),
            Self::Adopted(token) => token.cancel(),
        }
    }
}

struct Entry {
    info: TaskInfo,
    child: Option<Killer>,
}

/// Tasks of every session in this process.
pub struct TaskService {
    agent_dir: PathBuf,
    processes: ProcessService,
    tasks: Mutex<HashMap<String, Entry>>,
    finished: Mutex<HashMap<String, VecDeque<String>>>,
    next: Mutex<u64>,
}

impl std::fmt::Debug for TaskService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskService")
            .field("agent_dir", &self.agent_dir)
            .finish_non_exhaustive()
    }
}

fn session8(sid: &str) -> String {
    sid.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect()
}

impl TaskService {
    /// Service rooted at `agent_dir`.
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            agent_dir: agent_dir.to_path_buf(),
            processes: ProcessService::default(),
            tasks: Mutex::new(HashMap::new()),
            finished: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
        }
    }

    /// Starts a task for `owner` in `session_id`.
    ///
    /// # Errors
    ///
    /// Spawn failures.
    #[allow(
        clippy::too_many_lines,
        reason = "spawn and completion registration form one transaction"
    )]
    pub fn start(
        self: &Arc<Self>,
        owner: &ExtensionId,
        session_id: &str,
        cwd: &Path,
        mut spec: TaskSpec,
        on_finish: OnFinish,
    ) -> Result<String, HostError> {
        if spec.external_id.is_some() {
            return self.register_external(owner, session_id, spec);
        }
        if let Some(timeout) = spec.timeout_seconds
            && (!timeout.is_finite() || timeout <= 0.0 || timeout > 2_147_483.647)
        {
            return Err(HostError::InvalidArgument("Invalid timeout".to_owned()));
        }
        let (program, args) = spec.process_invocation()?;
        let id = self.next_id();
        let output = self.output_path(session_id, &id)?;
        let child = self.processes.spawn(&Spawn {
            command: program,
            args,
            cwd: cwd.to_path_buf(),
            env: spec.execution.as_ref().map_or_else(
                || vec![("OINO_TASK_ID".into(), id.clone())],
                |e| e.vars.clone(),
            ),
            clear_env: spec.execution.is_some(),
            stdin: if spec.execution.as_ref().is_some_and(|e| e.stdin.is_some()) {
                Input::Pipe
            } else {
                Input::Null
            },
            stdout: Output::File(output.clone()),
            stderr: Output::File(output.clone()),
            new_process_group: true,
            detached: false,
            timeout: spec.timeout_seconds.map(Duration::from_secs_f64),
        })?;
        if let Some(input) = spec.execution.as_ref().and_then(|e| e.stdin.clone()) {
            let child = Arc::clone(&child);
            tokio::spawn(async move {
                let _ = child.write_stdin(input.as_bytes()).await;
                child.close_stdin();
            });
        }
        let max = spec
            .max_output_bytes
            .unwrap_or(if spec.shell.is_some() {
                u64::MAX
            } else {
                DEFAULT_MAX_OUTPUT
            })
            .max(MIN_MAX_OUTPUT);
        // Never persist inherited environment values (which may contain credentials).
        spec.execution = None;
        let info = TaskInfo {
            id: id.clone(),
            owner: owner.to_string(),
            session_id: session_id.to_owned(),
            spec,
            state: TaskState::Running,
            exit_code: None,
            error: None,
            output_path: output.to_string_lossy().into_owned(),
            started_at: crate::timers::now_ms(),
            finished_at: None,
            pid: child.pid,
        };
        lock(&self.tasks).insert(
            id.clone(),
            Entry {
                info,
                child: Some(Killer::Child(Arc::clone(&child))),
            },
        );
        let this = Arc::clone(self);
        let task_id = id.clone();
        tokio::spawn(async move {
            let mut over_cap = false;
            let exit = loop {
                tokio::select! {
                    exit = child.wait() => break exit,
                    () = tokio::time::sleep(Duration::from_millis(200)) => {
                        let size = std::fs::metadata(&output).map_or(0, |m| m.len());
                        if size > max && !over_cap {
                            over_cap = true;
                            child.kill();
                        }
                    }
                }
            };
            if let Some(info) = this.finish(
                &task_id,
                exit,
                over_cap.then(|| format!("output exceeded {max} bytes")),
            ) {
                on_finish(info);
            }
        });
        Ok(id)
    }

    /// A fresh task id.
    #[must_use]
    pub fn next_id(&self) -> String {
        let mut next = lock(&self.next);
        *next += 1;
        format!("t{:x}{:04x}", crate::timers::now_ms() & 0xffff_ffff, *next)
    }

    fn register_external(
        &self,
        owner: &ExtensionId,
        session_id: &str,
        mut spec: TaskSpec,
    ) -> Result<String, HostError> {
        let id = spec.external_id.clone().unwrap_or_default();
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(HostError::InvalidArgument(
                "Invalid external task id".to_owned(),
            ));
        }
        let mut tasks = lock(&self.tasks);
        if let Some(old) = tasks.get(&id)
            && (old.info.owner != owner.as_str()
                || old.info.session_id != session_id
                || old.info.spec.external_id.is_none())
        {
            return Err(HostError::InvalidArgument(
                "Task id already registered".to_owned(),
            ));
        }
        let output = self.output_path(session_id, &id)?;
        if let Some(text) = &spec.external_output {
            std::fs::write(&output, text).map_err(|e| HostError::io(&e))?;
        } else if !output.exists() {
            std::fs::write(&output, "").map_err(|e| HostError::io(&e))?;
        }
        // Reports live in the log, not duplicated in every status response.
        spec.external_output = None;
        let state = spec.external_state.unwrap_or(TaskState::Running);
        let started_at = tasks
            .get(&id)
            .map_or_else(crate::timers::now_ms, |e| e.info.started_at);
        tasks.insert(
            id.clone(),
            Entry {
                info: TaskInfo {
                    id: id.clone(),
                    owner: owner.to_string(),
                    session_id: session_id.to_owned(),
                    spec,
                    state,
                    exit_code: (state == TaskState::Done).then_some(0),
                    error: None,
                    output_path: output.to_string_lossy().into_owned(),
                    started_at,
                    finished_at: (state != TaskState::Running).then(crate::timers::now_ms),
                    pid: 0,
                },
                child: None,
            },
        );
        drop(tasks);
        self.persist(session_id);
        Ok(id)
    }

    fn output_path(&self, session_id: &str, id: &str) -> Result<PathBuf, HostError> {
        let dir = self
            .agent_dir
            .join("tmp")
            .join("tasks")
            .join(format!("{}-{id}", session8(session_id)));
        std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&e))?;
        Ok(dir.join("output.log"))
    }

    /// Adopts a detached `bash` call as task `id` of `owner`: the output so far starts the
    /// log, further output is appended, completion is reported like a started task.
    ///
    /// # Errors
    ///
    /// The log file cannot be created.
    pub fn adopt(
        self: &Arc<Self>,
        id: &str,
        owner: &ExtensionId,
        session_id: &str,
        detached: oino_extension_api::DetachedBash,
        on_finish: OnFinish,
    ) -> Result<(), HostError> {
        use std::io::Write as _;
        let output = self.output_path(session_id, id)?;
        let mut file = std::fs::File::create(&output).map_err(|e| HostError::io(&e))?;
        if let Some(path) = &detached.full_output_path {
            let mut previous = std::fs::File::open(path).map_err(|e| HostError::io(&e))?;
            std::io::copy(&mut previous, &mut file).map_err(|e| HostError::io(&e))?;
        } else {
            file.write_all(detached.output_so_far.as_bytes())
                .map_err(|e| HostError::io(&e))?;
        }
        let spec = TaskSpec {
            name: format!(
                "bash: {}",
                detached.command.lines().next().unwrap_or_default()
            ),
            command: detached.command.clone(),
            is_agent: false,
            shell: None,
            execution: None,
            risk_level: None,
            arguments: None,
            external_id: None,
            external_state: None,
            external_output: None,
            description: detached
                .full_output_path
                .as_ref()
                .map(|p| format!("adopted bash call (full output so far: {p})")),
            timeout_seconds: None,
            notify_on_completion: true,
            trigger_on_completion: true,
            max_output_bytes: None,
        };
        let info = TaskInfo {
            id: id.to_owned(),
            owner: owner.to_string(),
            session_id: session_id.to_owned(),
            spec,
            state: TaskState::Running,
            exit_code: None,
            error: None,
            output_path: output.to_string_lossy().into_owned(),
            started_at: crate::timers::now_ms(),
            finished_at: None,
            pid: detached.pid.unwrap_or_default(),
        };
        lock(&self.tasks).insert(
            id.to_owned(),
            Entry {
                info,
                child: Some(Killer::Adopted(detached.kill.clone())),
            },
        );
        let this = Arc::clone(self);
        let task_id = id.to_owned();
        let mut chunks = detached.output;
        let exit = detached.exit;
        tokio::spawn(async move {
            use futures::StreamExt as _;
            let pump = async move {
                while let Some(chunk) = chunks.next().await {
                    let _ = file.write_all(&chunk);
                }
            };
            let pump = tokio::spawn(pump);
            let code = exit.await;
            // Descendants may keep the pipes open; stop reading shortly after the exit.
            let _ = tokio::time::timeout(Duration::from_secs(2), pump).await;
            let exit = crate::process::Exit {
                code,
                signal: None,
                timed_out: false,
            };
            if let Some(info) = this.finish(&task_id, exit, None) {
                on_finish(info);
            }
        });
        Ok(())
    }

    fn finish(
        &self,
        id: &str,
        exit: crate::process::Exit,
        error: Option<String>,
    ) -> Option<TaskInfo> {
        let info = {
            let mut tasks = lock(&self.tasks);
            let entry = tasks.get_mut(id)?;
            entry.child = None;
            let info = &mut entry.info;
            info.exit_code = exit.code;
            info.finished_at = Some(crate::timers::now_ms());
            if info.state != TaskState::Killed {
                info.state = if error.is_none() && !exit.timed_out && exit.code == Some(0) {
                    TaskState::Done
                } else {
                    TaskState::Failed
                };
            }
            info.error = error.or_else(|| exit.timed_out.then(|| "timed out".to_owned()));
            info.clone()
        };
        let evicted = {
            let mut finished = lock(&self.finished);
            let list = finished.entry(info.session_id.clone()).or_default();
            list.push_back(id.to_owned());
            let mut evicted = Vec::new();
            while list.len() > KEEP_FINISHED {
                evicted.extend(list.pop_front());
            }
            evicted
        };
        {
            let mut tasks = lock(&self.tasks);
            for old in evicted {
                tasks.remove(&old);
            }
        }
        self.persist(&info.session_id);
        Some(info)
    }

    fn persist(&self, session_id: &str) {
        let list = self.list(session_id);
        let path = self
            .agent_dir
            .join("state")
            .join("oino.tasks")
            .join("session")
            .join(session8(session_id))
            .join("tasks.json");
        if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
            let _ = crate::kv::atomic_write(&path, &bytes, None);
        }
    }

    /// Tasks of `session_id` (running + last 30 finished), oldest first.
    #[must_use]
    pub fn list(&self, session_id: &str) -> Vec<TaskInfo> {
        let mut list: Vec<TaskInfo> = lock(&self.tasks)
            .values()
            .filter(|e| e.info.session_id == session_id)
            .map(|e| e.info.clone())
            .collect();
        list.sort_by_key(|t| t.started_at);
        list
    }

    /// One task.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<TaskInfo> {
        lock(&self.tasks).get(id).map(|e| e.info.clone())
    }

    /// Output text (at most `max_bytes`, from the end when `tail`).
    ///
    /// # Errors
    ///
    /// Unknown task, I/O.
    pub fn logs(&self, id: &str, max_bytes: u32, tail: bool) -> Result<String, HostError> {
        let info = self
            .get(id)
            .ok_or_else(|| HostError::NotFound(format!("task {id}")))?;
        let mut file = std::fs::File::open(&info.output_path).map_err(|e| HostError::io(&e))?;
        let size = file.metadata().map_err(|e| HostError::io(&e))?.len();
        let max = u64::from(max_bytes).min(size);
        if tail {
            file.seek(SeekFrom::Start(size - max))
                .map_err(|e| HostError::io(&e))?;
        }
        let mut bytes = Vec::new();
        file.take(max)
            .read_to_end(&mut bytes)
            .map_err(|e| HostError::io(&e))?;
        if tail {
            let skip = bytes.iter().take_while(|b| (**b & 0xc0) == 0x80).count();
            bytes.drain(..skip);
        } else {
            while std::str::from_utf8(&bytes).is_err_and(|e| e.error_len().is_none()) {
                bytes.pop();
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Kills a running task.
    ///
    /// # Errors
    ///
    /// Unknown task.
    pub fn kill(&self, id: &str) -> Result<(), HostError> {
        let mut tasks = lock(&self.tasks);
        let entry = tasks
            .get_mut(id)
            .ok_or_else(|| HostError::NotFound(format!("task {id}")))?;
        if let Some(killer) = &entry.child {
            entry.info.state = TaskState::Killed;
            killer.kill();
        }
        Ok(())
    }

    /// Drops finished tasks of `session_id`.
    pub fn clear_finished(&self, session_id: &str) {
        let ids: Vec<String> = lock(&self.finished)
            .remove(session_id)
            .map(Vec::from)
            .unwrap_or_default();
        let mut tasks = lock(&self.tasks);
        for id in ids {
            tasks.remove(&id);
        }
    }

    /// Running tasks of `session_id` (arbiter wait sources).
    #[must_use]
    pub fn running(&self, session_id: &str) -> usize {
        lock(&self.tasks)
            .values()
            .filter(|e| e.info.session_id == session_id && e.info.state == TaskState::Running)
            .count()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_logs_and_notifies() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(TaskService::new(dir.path()));
        let owner = ExtensionId::new("t").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let spec: TaskSpec = serde_json::from_value(
            serde_json::json!({"name": "echo", "command": "echo hello; exit 2"}),
        )
        .unwrap();
        let id = svc
            .start(
                &owner,
                "s1",
                dir.path(),
                spec,
                Arc::new(move |i| {
                    let _ = tx.send(i);
                }),
            )
            .unwrap();
        let info = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (info.id.as_str(), info.state, info.exit_code),
            (id.as_str(), TaskState::Failed, Some(2))
        );
        assert_eq!(svc.logs(&id, 100, true).unwrap(), "hello\n");
        assert_eq!(svc.list("s1").len(), 1);
        assert_eq!(svc.running("s1"), 0);
        svc.clear_finished("s1");
        assert!(svc.list("s1").is_empty());
    }

    #[tokio::test]
    async fn kill_and_output_cap() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(TaskService::new(dir.path()));
        let owner = ExtensionId::new("t").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let spec: TaskSpec =
            serde_json::from_value(serde_json::json!({"name": "s", "command": "sleep 30"}))
                .unwrap();
        let tx2 = tx.clone();
        let id = svc
            .start(
                &owner,
                "s",
                dir.path(),
                spec,
                Arc::new(move |i| {
                    let _ = tx2.send(i);
                }),
            )
            .unwrap();
        svc.kill(&id).unwrap();
        let info = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.state, TaskState::Killed);
        let spec: TaskSpec = serde_json::from_value(
            serde_json::json!({"name": "y", "command": "while true; do echo 0123456789012345678901234567890123456789; done", "maxOutputBytes": 2048}),
        )
        .unwrap();
        svc.start(
            &owner,
            "s",
            dir.path(),
            spec,
            Arc::new(move |i| {
                let _ = tx.send(i);
            }),
        )
        .unwrap();
        let info = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.state, TaskState::Failed);
        assert!(info.error.unwrap().contains("output exceeded"));
    }

    #[tokio::test]
    async fn adopts_a_detached_bash_call() {
        let dir = tempfile::tempdir().unwrap();
        let svc = Arc::new(TaskService::new(dir.path()));
        let owner = ExtensionId::new("t").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (chunks, output) = futures::channel::mpsc::unbounded::<Vec<u8>>();
        let kill = tokio_util::sync::CancellationToken::new();
        let killed = kill.clone();
        let detached = oino_extension_api::DetachedBash {
            command: "sleep 99".into(),
            pid: Some(42),
            output_so_far: "before".into(),
            full_output_path: None,
            output: Box::pin(output),
            exit: Box::pin(async move {
                killed.cancelled().await;
                None
            }),
            kill,
        };
        let id = svc.next_id();
        svc.adopt(
            &id,
            &owner,
            "s",
            detached,
            Arc::new(move |i| {
                let _ = tx.send(i);
            }),
        )
        .unwrap();
        chunks.unbounded_send(b"after\n".to_vec()).unwrap();
        drop(chunks);
        let info = svc.get(&id).unwrap();
        assert_eq!((info.state, info.pid), (TaskState::Running, 42));
        assert_eq!(info.spec.name, "bash: sleep 99");
        assert_eq!(svc.running("s"), 1);
        tokio::time::sleep(Duration::from_millis(50)).await;
        svc.kill(&id).unwrap();
        let info = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.state, TaskState::Killed);
        assert_eq!(svc.logs(&id, 100, false).unwrap(), "beforeafter\n");
    }
}
