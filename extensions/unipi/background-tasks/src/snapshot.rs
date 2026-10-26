//! unipi `BgTaskSnapshot`, built from the host's `TaskInfo` (extension-api §5.5) plus the
//! state this extension keeps per task (cwd, notified, output size and tail).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::text;

/// Output tail lines kept for cards (unipi `OUTPUT_TAIL_LINES`).
pub const OUTPUT_TAIL_LINES: usize = 3;
/// Extension id (tasks started by other owners count as adopted).
pub const EXTENSION_ID: &str = "unipi.background-tasks";

/// unipi `TaskStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Running.
    #[default]
    Running,
    /// Exit 0.
    Completed,
    /// Non-zero exit, timeout, output cap, spawn error.
    Failed,
    /// Stopped on request.
    Killed,
}

impl Status {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }

    /// Host `TaskState` (`running|done|failed|killed`) → unipi status.
    #[must_use]
    pub fn from_host(state: &str) -> Self {
        match state {
            "done" => Self::Completed,
            "failed" => Self::Failed,
            "killed" => Self::Killed,
            _ => Self::Running,
        }
    }
}

/// The host `TaskInfo` fields this extension reads.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostTask {
    /// Task id.
    pub id: String,
    /// Owning extension.
    pub owner: String,
    /// Display name.
    pub name: String,
    /// Shell command.
    pub command: String,
    /// Launches an agent.
    pub is_agent: bool,
    /// Description.
    pub description: Option<String>,
    /// Timeout.
    pub timeout_seconds: Option<u64>,
    /// Notify on completion.
    pub notify_on_completion: bool,
    /// Trigger a turn on completion.
    pub trigger_on_completion: bool,
    /// `running|done|failed|killed`.
    pub state: String,
    /// Exit code.
    pub exit_code: Option<i32>,
    /// Failure reason.
    pub error: Option<String>,
    /// Host output path.
    pub output_path: String,
    /// Start (epoch ms).
    pub started_at: i64,
    /// End (epoch ms).
    pub finished_at: Option<i64>,
    /// Process id.
    pub pid: u32,
}

impl HostTask {
    /// Decodes a host `TaskInfo` JSON value.
    #[must_use]
    pub fn decode(v: &Value) -> Option<Self> {
        serde_json::from_value::<Self>(v.clone())
            .ok()
            .filter(|t| !t.id.is_empty())
    }
}

/// What this extension knows about a task beyond the host record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Local {
    /// cwd at start.
    pub cwd: String,
    /// Notification delivered.
    pub notified: bool,
    /// Stop reason recorded here (shutdown, watchdog).
    pub error: Option<String>,
    /// Stopped on request (`stop`/`kill`), so a non-zero exit is `killed`.
    pub stopped: bool,
}

/// unipi `BgTaskSnapshot` (telemetry fields are absent: Oino tasks are not telemetry-wrapped).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi BgTaskSnapshot"
)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    /// Task id.
    pub id: String,
    /// Display name (always the resolved name, like unipi `snapshot()`).
    pub name: Option<String>,
    /// Command.
    pub command: String,
    /// Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Status.
    pub status: Status,
    /// Output file (host path).
    pub output_path: String,
    /// cwd.
    pub cwd: String,
    /// Start (epoch ms).
    pub start_time: i64,
    /// End (epoch ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    /// Exit code (`Some(None)` = finished without one, serialised `null`).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    #[allow(
        clippy::option_option,
        reason = "unipi exitCode is number | null | absent"
    )]
    pub exit_code: Option<Option<i32>>,
    /// Process id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Output bytes written.
    pub bytes_written: u64,
    /// Agent task.
    pub is_agent: bool,
    /// Error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Notification delivered.
    pub notified: bool,
    /// Notify on completion.
    pub notify_on_completion: bool,
    /// Wake on completion.
    pub trigger_on_completion: bool,
    /// Timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Why agent telemetry is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_unavailable_reason: Option<String>,
    /// Last output lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tail: Option<Vec<String>>,
    /// Not started by this extension (watchdog/bash-detach adoption).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watchdog_adopted: Option<bool>,
}

/// A present field (even `null`) is `Some`.
#[allow(
    clippy::option_option,
    reason = "unipi exitCode is number | null | absent"
)]
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i32>>, D::Error> {
    Option::<i32>::deserialize(d).map(Some)
}

/// unipi `telemetryUnavailableReason` for agent tasks in Oino.
pub const NO_TELEMETRY: &str =
    "agent telemetry wrapping is not available for host background tasks";

impl Snapshot {
    /// Builds the snapshot from the host record, local state and the current output.
    #[must_use]
    pub fn build(host: &HostTask, local: &Local, output: Option<&str>, default_cwd: &str) -> Self {
        let mut status = Status::from_host(&host.state);
        if local.stopped && status == Status::Failed {
            status = Status::Killed;
        }
        let finished = status != Status::Running;
        let error = if local.stopped {
            local.error.clone()
        } else {
            local.error.clone().or_else(|| {
                host.error
                    .as_deref()
                    .map(|e| map_host_error(e, host.timeout_seconds))
                    .or_else(|| {
                        (status == Status::Failed).then(|| {
                            format!(
                                "Exited with code {}",
                                host.exit_code
                                    .map_or_else(|| "null".to_owned(), |c| c.to_string())
                            )
                        })
                    })
            })
        };
        Self {
            id: host.id.clone(),
            name: Some(text::task_display_name(
                Some(&host.name),
                host.description.as_deref(),
                Some(&host.command),
                Some(&host.id),
            )),
            command: host.command.clone(),
            description: host.description.clone(),
            status,
            output_path: host.output_path.clone(),
            cwd: if local.cwd.is_empty() {
                default_cwd.to_owned()
            } else {
                local.cwd.clone()
            },
            start_time: host.started_at,
            end_time: host.finished_at,
            exit_code: finished.then_some(host.exit_code),
            pid: (host.pid > 0).then_some(host.pid),
            bytes_written: output.map_or(0, |o| o.len() as u64),
            is_agent: host.is_agent,
            error,
            notified: local.notified,
            notify_on_completion: host.notify_on_completion,
            trigger_on_completion: host.trigger_on_completion,
            timeout_seconds: host.timeout_seconds,
            telemetry_unavailable_reason: host.is_agent.then(|| NO_TELEMETRY.to_owned()),
            output_tail: output
                .map(|o| text::output_tail(o, OUTPUT_TAIL_LINES))
                .filter(|t| !t.is_empty()),
            watchdog_adopted: (!host.owner.is_empty() && host.owner != EXTENSION_ID)
                .then_some(true),
        }
    }

    /// unipi `taskDisplayName` of this snapshot.
    #[must_use]
    pub fn display_name(&self) -> String {
        text::task_display_name(
            self.name.as_deref(),
            self.description.as_deref(),
            Some(&self.command),
            Some(&self.id),
        )
    }
}

/// Host error texts → unipi's (`timed out` → `Timed out after Ns`, output cap).
#[must_use]
pub fn map_host_error(error: &str, timeout_seconds: Option<u64>) -> String {
    if error == "timed out" {
        return timeout_seconds.map_or_else(
            || "Timed out".to_owned(),
            |s| format!("Timed out after {s}s"),
        );
    }
    if let Some(n) = error
        .strip_prefix("output exceeded ")
        .and_then(|r| r.strip_suffix(" bytes"))
        .and_then(|n| n.parse::<u64>().ok())
    {
        return format!("Output exceeded cap of {}", text::format_size(n));
    }
    error.to_owned()
}

/// unipi task resolution: exact id, else a unique prefix.
///
/// # Errors
///
/// unipi's messages (`Task ID is required`, ambiguous prefix, unknown id).
pub fn resolve<'a>(tasks: &'a [HostTask], id_or_prefix: &str) -> Result<&'a HostTask, String> {
    let id = id_or_prefix.trim();
    if id.is_empty() {
        return Err("Task ID is required".to_owned());
    }
    if let Some(t) = tasks.iter().find(|t| t.id == id) {
        return Ok(t);
    }
    let matches: Vec<&HostTask> = tasks.iter().filter(|t| t.id.starts_with(id)).collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("Unknown background task ID: {id}")),
        many => Err(format!(
            "Ambiguous task ID prefix \"{id}\": {}",
            many.iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn host(state: &str, code: Option<i32>) -> HostTask {
        HostTask::decode(&json!({
            "id": "t1", "owner": EXTENSION_ID, "sessionId": "s", "name": "tests", "command": "npm test",
            "isAgent": false, "notifyOnCompletion": true, "triggerOnCompletion": true, "state": state,
            "exitCode": code, "outputPath": "/o/output.log", "startedAt": 1000, "finishedAt": 6000, "pid": 42
        }))
        .unwrap_or_default()
    }

    #[test]
    #[allow(clippy::many_single_char_names, reason = "short test bindings")]
    fn maps_host_states_and_errors() {
        let s = Snapshot::build(
            &host("done", Some(0)),
            &Local::default(),
            Some("a\nb\n"),
            "/w",
        );
        assert_eq!(s.status, Status::Completed);
        assert_eq!(s.exit_code, Some(Some(0)));
        assert_eq!(s.bytes_written, 4);
        assert_eq!(s.cwd, "/w");
        assert_eq!(s.output_tail, Some(vec!["a".into(), "b".into()]));
        assert_eq!(s.watchdog_adopted, None);
        let f = Snapshot::build(&host("failed", Some(2)), &Local::default(), None, "/w");
        assert_eq!(f.error.as_deref(), Some("Exited with code 2"));
        let stopped = Local {
            stopped: true,
            ..Local::default()
        };
        let k = Snapshot::build(&host("failed", None), &stopped, None, "/w");
        assert_eq!((k.status, k.error.as_deref()), (Status::Killed, None));
        let r = Snapshot::build(&host("running", None), &Local::default(), None, "/w");
        assert_eq!(r.exit_code, None);
        let v = serde_json::to_value(&k).unwrap_or_default();
        assert_eq!(v["exitCode"], Value::Null);
        assert!(v.as_object().is_some_and(|o| o.contains_key("exitCode")));
        assert_eq!(v["status"], "killed");
        assert_eq!(map_host_error("timed out", Some(5)), "Timed out after 5s");
        assert_eq!(
            map_host_error("output exceeded 1048576 bytes", None),
            "Output exceeded cap of 1.0MB"
        );
        let mut adopted = host("running", None);
        adopted.owner = "unipi.watchdog".into();
        assert_eq!(
            Snapshot::build(&adopted, &Local::default(), None, "/").watchdog_adopted,
            Some(true)
        );
    }

    #[test]
    fn resolves_ids_and_prefixes() {
        let mut a = host("running", None);
        a.id = "tabc1".into();
        let mut b = host("running", None);
        b.id = "tabc2".into();
        let tasks = vec![a, b];
        assert_eq!(resolve(&tasks, "tabc1").map(|t| t.id.as_str()), Ok("tabc1"));
        assert_eq!(resolve(&tasks, " ").unwrap_err(), "Task ID is required");
        assert!(
            resolve(&tasks, "tab")
                .unwrap_err()
                .starts_with("Ambiguous task ID prefix \"tab\": tabc1, tabc2")
        );
        assert_eq!(
            resolve(&tasks, "zz").unwrap_err(),
            "Unknown background task ID: zz"
        );
    }
}
