//! Host background-task service (C12, extension-api §5.5).

use std::collections::{HashMap, VecDeque};
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

/// `TaskSpec` (unipi background-tasks).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// Launches an agent process.
    #[serde(default)]
    pub is_agent: bool,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Kill after this many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
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

struct Entry {
    info: TaskInfo,
    child: Option<Arc<crate::process::ChildHandle>>,
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
    pub fn start(
        self: &Arc<Self>,
        owner: &ExtensionId,
        session_id: &str,
        cwd: &Path,
        spec: TaskSpec,
        on_finish: OnFinish,
    ) -> Result<String, HostError> {
        let id = {
            let mut next = lock(&self.next);
            *next += 1;
            format!("t{:x}{:04x}", crate::timers::now_ms() & 0xffff_ffff, *next)
        };
        let dir = self
            .agent_dir
            .join("tmp")
            .join("tasks")
            .join(format!("{}-{id}", session8(session_id)));
        std::fs::create_dir_all(&dir).map_err(|e| HostError::io(&e))?;
        let output = dir.join("output.log");
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let child = self.processes.spawn(&Spawn {
            command: shell.into(),
            args: vec![flag.into(), spec.command.clone()],
            cwd: cwd.to_path_buf(),
            env: vec![("OINO_TASK_ID".into(), id.clone())],
            clear_env: false,
            stdin: Input::Null,
            stdout: Output::File(output.clone()),
            stderr: Output::File(output.clone()),
            new_process_group: true,
            detached: false,
            timeout: spec.timeout_seconds.map(Duration::from_secs),
        })?;
        let max = spec
            .max_output_bytes
            .unwrap_or(DEFAULT_MAX_OUTPUT)
            .max(MIN_MAX_OUTPUT);
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
                child: Some(Arc::clone(&child)),
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
        let bytes = std::fs::read(&info.output_path).unwrap_or_default();
        let max = usize::try_from(max_bytes)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let slice = if tail {
            &bytes[bytes.len() - max..]
        } else {
            &bytes[..max]
        };
        Ok(String::from_utf8_lossy(slice).into_owned())
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
        if let Some(child) = &entry.child {
            entry.info.state = TaskState::Killed;
            child.kill();
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
}
