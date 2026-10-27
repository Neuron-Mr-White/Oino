//! The CLI's JSON contract (unipi `shapes.ts`).
//!
//! Every shape the extension consumes is validated here instead of being assumed, so a
//! change on the binary side fails loudly with a clear message.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A shape mismatch between the extension and the binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError {
    /// Command whose output did not match.
    pub command: String,
    /// What was wrong.
    pub detail: String,
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unexpected {} output: {} — the extension and the binary disagree (version mismatch?)",
            self.command, self.detail
        )
    }
}

fn shape(command: &str, detail: impl Into<String>) -> ShapeError {
    ShapeError {
        command: command.to_owned(),
        detail: detail.into(),
    }
}

/// A claim block (`run`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Claiming session.
    #[serde(default)]
    pub session: Option<String>,
    /// Claiming pid.
    #[serde(default)]
    pub pid: Option<u64>,
    /// Host of the claiming process.
    #[serde(default)]
    pub host: Option<String>,
    /// `agent` (claimed with `start`).
    #[serde(default)]
    pub owner: Option<String>,
}

/// One activity line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    /// Timestamp.
    #[serde(default)]
    pub at: String,
    /// Actor.
    #[serde(default)]
    pub actor: String,
    /// Text.
    #[serde(default)]
    pub text: String,
}

/// A task as `list`/`show`/`add`/`move` print it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Id.
    pub id: String,
    /// Raw title (may be empty).
    pub title: String,
    /// Title derived from the first body line when `title` is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    /// Status.
    pub status: String,
    /// Priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Lane order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// Dependencies.
    #[serde(default)]
    pub deps: Vec<String>,
    /// Claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Run>,
    /// Activity, oldest first.
    #[serde(default)]
    pub activity: Vec<Activity>,
    /// Ready to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    /// Unmet dependencies.
    #[serde(default)]
    pub waiting_for: Vec<String>,
    /// Staleness of the claim (`fresh`, `stale`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staleness: Option<String>,
    /// Moves the acting user may make.
    #[serde(default)]
    pub allowed_moves: Vec<String>,
    /// Every other field, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Task {
    /// The title a row shows: the derived one when the raw title is empty.
    #[must_use]
    pub fn shown_title(&self) -> &str {
        match self.display_title.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ if !self.title.is_empty() => &self.title,
            _ => "(untitled)",
        }
    }

    /// In progress and claimed by `session`.
    #[must_use]
    pub fn claimed_by(&self, session: &str) -> bool {
        self.status == "in_progress"
            && self
                .run
                .as_ref()
                .and_then(|r| r.session.as_deref())
                .is_some_and(|s| s == session)
    }

    /// The blocked reason text, when the binary reports one.
    #[must_use]
    pub fn blocked_reason(&self) -> Option<&str> {
        self.extra
            .get("blockedReason")
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    }

    /// Attachment count reported by the binary.
    #[must_use]
    pub fn attachment_count(&self) -> usize {
        self.extra
            .get("attachments")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    }
}

fn require_object<'a>(
    command: &str,
    value: &'a Value,
) -> Result<&'a Map<String, Value>, ShapeError> {
    match value {
        Value::Object(map) => Ok(map),
        Value::Array(_) => Err(shape(command, "expected an object, got an array")),
        other => Err(shape(
            command,
            format!("expected an object, got {}", type_name(other)),
        )),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Null | Value::Object(_) => "object",
    }
}

/// `show …, add …, move …, note …, release …` → one task.
pub fn as_task(command: &str, value: &Value) -> Result<Task, ShapeError> {
    let map = require_object(command, value)?;
    for key in ["id", "title", "status"] {
        if !map.get(key).is_some_and(Value::is_string) {
            return Err(shape(command, format!("missing string \"{key}\"")));
        }
    }
    serde_json::from_value(value.clone()).map_err(|e| shape(command, e.to_string()))
}

/// One problem `list`/`validate` report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// File.
    pub file: String,
    /// Line.
    pub line: u64,
    /// Message.
    pub error: String,
    /// `validate --fix` can repair it.
    pub fixable: bool,
}

/// `list --json` → `{tasks, problems}`.
pub fn as_task_list(value: &Value) -> Result<(Vec<Task>, Vec<Problem>), ShapeError> {
    if value.is_array() {
        return Err(shape(
            "list",
            "an array here means the binary is older than the extension",
        ));
    }
    let map = require_object("list", value)?;
    let Some(tasks) = map.get("tasks").and_then(Value::as_array) else {
        return Err(shape("list", "expected {tasks: [...], problems: [...]}"));
    };
    let tasks = tasks
        .iter()
        .map(|t| as_task("list", t))
        .collect::<Result<Vec<_>, _>>()?;
    let problems = map
        .get("problems")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|p| Problem {
                    file: p
                        .get("file")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    line: p.get("line").and_then(Value::as_u64).unwrap_or(0),
                    error: p
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    fixable: p.get("fixable").and_then(Value::as_bool) == Some(true),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((tasks, problems))
}

/// A project (`project add/list`, `project show` → `project`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    /// Slug.
    pub slug: String,
    /// Display name.
    pub name: String,
}

/// `project add` → a project.
pub fn as_project(command: &str, value: &Value) -> Result<Project, ShapeError> {
    let map = require_object(command, value)?;
    let get = |key: &str| {
        map.get(key)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| shape(command, format!("missing string \"{key}\"")))
    };
    Ok(Project {
        slug: get("slug")?,
        name: get("name")?,
    })
}

/// `project show` → `{project, counts, total, problems}`.
pub fn as_project_detail(value: &Value) -> Result<(Project, u64), ShapeError> {
    let map = require_object("project show", value)?;
    let project = as_project("project show", map.get("project").unwrap_or(&Value::Null))?;
    Ok((
        project,
        map.get("total").and_then(Value::as_u64).unwrap_or(0),
    ))
}

/// `stop` → `{stopped, pid?, reason?}`.
pub fn as_stop(value: &Value) -> Result<(bool, Option<u64>, Option<String>), ShapeError> {
    let map = require_object("stop", value)?;
    let Some(stopped) = map.get("stopped").and_then(Value::as_bool) else {
        return Err(shape("stop", "missing boolean \"stopped\""));
    };
    Ok((
        stopped,
        map.get("pid").and_then(Value::as_u64),
        map.get("reason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    ))
}

/// `daemon.json` as `status` reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    /// Pid.
    pub pid: u64,
    /// Port.
    pub port: u16,
    /// Version.
    #[serde(default)]
    pub version: String,
    /// Bind host.
    #[serde(default)]
    pub host: Option<String>,
    /// Access token (non-loopback or `--require-auth`).
    #[serde(default)]
    pub token: Option<String>,
}

/// `status` → `{daemon, alive, home, binary, hostname}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The recorded daemon.
    pub daemon: Option<DaemonInfo>,
    /// Its pid is alive.
    pub alive: bool,
    /// Kanboard home.
    pub home: Option<String>,
    /// Absolute path of the binary.
    pub binary: Option<String>,
    /// This machine's host name.
    pub hostname: Option<String>,
}

/// Parses `status`.
pub fn as_status(value: &Value) -> Result<Status, ShapeError> {
    let map = require_object("status", value)?;
    let daemon = match map.get("daemon") {
        None | Some(Value::Null) => None,
        Some(d @ Value::Object(_)) => serde_json::from_value(d.clone()).ok(),
        Some(_) => return Err(shape("status", "\"daemon\" must be an object or null")),
    };
    let s = |key: &str| map.get(key).and_then(Value::as_str).map(ToOwned::to_owned);
    Ok(Status {
        daemon,
        alive: map.get("alive").and_then(Value::as_bool) == Some(true),
        home: s("home"),
        binary: s("binary"),
        hostname: s("hostname"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn task_requires_id_title_status() {
        let ok = as_task(
            "show",
            &json!({"id": "A-1", "title": "", "status": "todo", "x": 1}),
        )
        .unwrap();
        assert_eq!(ok.shown_title(), "(untitled)");
        assert_eq!(ok.extra["x"], 1);
        let err = as_task("show", &json!({"id": "A-1", "status": "todo"})).unwrap_err();
        assert_eq!(
            err.to_string(),
            "unexpected show output: missing string \"title\" — the extension and the binary disagree (version mismatch?)"
        );
        assert!(
            as_task("show", &json!([1]))
                .unwrap_err()
                .detail
                .contains("array")
        );
    }

    #[test]
    fn list_rejects_the_old_array_shape() {
        let err = as_task_list(&json!([])).unwrap_err();
        assert!(err.detail.contains("older than the extension"));
        let err = as_task_list(&json!({"x": 1})).unwrap_err();
        assert!(err.detail.contains("{tasks"));
        let (tasks, problems) = as_task_list(&json!({
            "tasks": [{"id": "A-1", "title": "t", "displayTitle": "t", "status": "in_progress", "run": {"session": "s1"}}],
            "problems": [{"file": "f", "line": 3, "error": "bad"}],
        }))
        .unwrap();
        assert!(tasks[0].claimed_by("s1"));
        assert!(!tasks[0].claimed_by("s2"));
        assert_eq!(problems[0].line, 3);
        assert!(!problems[0].fixable);
    }

    #[test]
    fn status_stop_and_project_shapes() {
        let s = as_status(&json!({"daemon": {"pid": 1, "port": 9, "version": "1"}, "alive": true, "binary": "/b"})).unwrap();
        assert_eq!(s.daemon.unwrap().port, 9);
        assert_eq!(s.binary.as_deref(), Some("/b"));
        assert!(as_status(&json!({"daemon": 3})).is_err());
        assert_eq!(
            as_stop(&json!({"stopped": false, "reason": "r"})).unwrap(),
            (false, None, Some("r".into()))
        );
        assert!(as_stop(&json!({})).is_err());
        let (p, total) =
            as_project_detail(&json!({"project": {"slug": "s", "name": "n"}, "total": 4})).unwrap();
        assert_eq!((p.slug.as_str(), total), ("s", 4));
    }
}
