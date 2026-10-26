//! Versioned bus API for other extensions (unipi `extension-api.ts`): requests on
//! [`REQUEST_CHANNEL`], replies on [`RESPONSE_CHANNEL`], task ends on [`TERMINAL_CHANNEL`]. The
//! channels are host bus topics (unipi-port §2.1 maps `pi.events` onto the bus).

use serde_json::{Map, Value, json};

/// Request topic.
pub const REQUEST_CHANNEL: &str = "unipi-background-tasks:request:v1";
/// Response topic.
pub const RESPONSE_CHANNEL: &str = "unipi-background-tasks:response:v1";
/// Terminal topic.
pub const TERMINAL_CHANNEL: &str = "unipi-background-tasks:terminal:v1";
/// Request schema.
pub const REQUEST_SCHEMA: &str = "unipi-background-tasks.extension-request.v1";
/// Response schema.
pub const RESPONSE_SCHEMA: &str = "unipi-background-tasks.extension-response.v1";
/// Terminal schema.
pub const TERMINAL_SCHEMA: &str = "unipi-background-tasks.extension-terminal.v1";

const MAX_ERROR_CHARS: usize = 240;
const MAX_REQUEST_ID_CHARS: usize = 200;

/// `capabilities` result.
#[must_use]
pub fn capabilities() -> Value {
    json!({
        "api_version": 1, "run": true, "run_is_agent": true, "run_completion_trigger": true,
        "status": true, "logs": true, "logs_bounded": true, "kill": true,
    })
}

/// A validated request.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// `capabilities`.
    Capabilities,
    /// `run`.
    Run {
        /// Name.
        name: String,
        /// Command.
        command: String,
        /// Agent task.
        is_agent: bool,
        /// Timeout.
        timeout_seconds: Option<u64>,
        /// Notify.
        notify_on_completion: bool,
        /// Wake.
        trigger_on_completion: bool,
    },
    /// `status`.
    Status {
        /// Task id.
        task_id: Option<String>,
    },
    /// `logs`.
    Logs {
        /// Task id.
        task_id: String,
        /// Byte cap.
        max_bytes: Option<u64>,
        /// Tail.
        tail: Option<bool>,
    },
    /// `kill`.
    Kill {
        /// Task id.
        task_id: String,
    },
}

impl Operation {
    /// Wire name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Run { .. } => "run",
            Self::Status { .. } => "status",
            Self::Logs { .. } => "logs",
            Self::Kill { .. } => "kill",
        }
    }
}

/// A parsed request frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Request id.
    pub request_id: String,
    /// Operation.
    pub operation: Operation,
}

/// A rejected frame: echoes for the error response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    /// Request id echo (`malformed` when absent).
    pub request_id: String,
    /// Operation echo.
    pub operation: String,
    /// Error text.
    pub error: String,
}

type Obj = Map<String, Value>;

fn record<'a>(v: &'a Value, label: &str) -> Result<&'a Obj, String> {
    v.as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}

fn closed(o: &Obj, allowed: &[&str], label: &str) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{label} contains unknown key {k}")),
        None => Ok(()),
    }
}

fn non_empty(v: Option<&Value>, label: &str) -> Result<String, String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} must be a non-empty string"))
}

fn boolean(v: Option<&Value>, label: &str) -> Result<bool, String> {
    v.and_then(Value::as_bool)
        .ok_or_else(|| format!("{label} must be boolean"))
}

fn positive(v: Option<&Value>, label: &str) -> Result<u64, String> {
    v.and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("{label} must be a positive integer"))
}

fn echo(v: Option<&Value>) -> String {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("malformed")
        .to_owned()
}

fn parse_payload(op: &str, v: &Value) -> Result<Operation, String> {
    let label = format!("{op}.payload");
    let p = record(v, &label)?;
    let field = |k: &str| format!("{label}.{k}");
    Ok(match op {
        "capabilities" => {
            closed(p, &[], &label)?;
            Operation::Capabilities
        }
        "run" => {
            closed(
                p,
                &[
                    "name",
                    "command",
                    "isAgent",
                    "timeoutSeconds",
                    "notifyOnCompletion",
                    "triggerOnCompletion",
                ],
                &label,
            )?;
            Operation::Run {
                name: non_empty(p.get("name"), &field("name"))?,
                command: non_empty(p.get("command"), &field("command"))?,
                is_agent: boolean(p.get("isAgent"), &field("isAgent"))?,
                notify_on_completion: boolean(
                    p.get("notifyOnCompletion"),
                    &field("notifyOnCompletion"),
                )?,
                trigger_on_completion: boolean(
                    p.get("triggerOnCompletion"),
                    &field("triggerOnCompletion"),
                )?,
                timeout_seconds: p
                    .contains_key("timeoutSeconds")
                    .then(|| positive(p.get("timeoutSeconds"), &field("timeoutSeconds")))
                    .transpose()?,
            }
        }
        "status" => {
            closed(p, &["taskId"], &label)?;
            Operation::Status {
                task_id: p
                    .contains_key("taskId")
                    .then(|| non_empty(p.get("taskId"), &field("taskId")))
                    .transpose()?,
            }
        }
        "logs" => {
            closed(p, &["taskId", "maxBytes", "tail"], &label)?;
            Operation::Logs {
                task_id: non_empty(p.get("taskId"), &field("taskId"))?,
                max_bytes: p
                    .contains_key("maxBytes")
                    .then(|| positive(p.get("maxBytes"), &field("maxBytes")))
                    .transpose()?,
                tail: p
                    .contains_key("tail")
                    .then(|| boolean(p.get("tail"), &field("tail")))
                    .transpose()?,
            }
        }
        _ => {
            closed(p, &["taskId"], &label)?;
            Operation::Kill {
                task_id: non_empty(p.get("taskId"), &field("taskId"))?,
            }
        }
    })
}

/// unipi `parseRequest`.
///
/// # Errors
///
/// [`Rejected`] with unipi's validation message.
pub fn parse_request(data: &Value) -> Result<Request, Rejected> {
    let Some(o) = data.as_object() else {
        return Err(Rejected {
            request_id: "malformed".into(),
            operation: "malformed".into(),
            error: "request frame must be an object".into(),
        });
    };
    let reject = |error: String| Rejected {
        request_id: echo(o.get("request_id")),
        operation: echo(o.get("operation")),
        error,
    };
    closed(
        o,
        &["schema_version", "request_id", "operation", "payload"],
        "request",
    )
    .map_err(reject)?;
    if o.get("schema_version").and_then(Value::as_str) != Some(REQUEST_SCHEMA) {
        return Err(reject("request schema_version mismatch".into()));
    }
    let request_id = non_empty(o.get("request_id"), "request.request_id").map_err(reject)?;
    if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
        return Err(reject(format!(
            "request.request_id must be at most {MAX_REQUEST_ID_CHARS} characters"
        )));
    }
    let op = o
        .get("operation")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "capabilities" | "run" | "status" | "logs" | "kill"))
        .ok_or_else(|| {
            reject("request.operation must be one of capabilities, run, status, logs, kill".into())
        })?;
    let payload = o
        .get("payload")
        .ok_or_else(|| reject("request.payload is required".into()))?;
    let operation = parse_payload(op, payload).map_err(|e| Rejected {
        request_id: request_id.clone(),
        operation: op.to_owned(),
        error: e,
    })?;
    Ok(Request {
        request_id,
        operation,
    })
}

/// unipi `boundedBackgroundTaskError`: whitespace collapsed, ≤ 240 chars.
#[must_use]
pub fn bounded_error(error: &str) -> String {
    let text = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= MAX_ERROR_CHARS {
        return text;
    }
    format!(
        "{}…",
        text.chars().take(MAX_ERROR_CHARS - 1).collect::<String>()
    )
}

/// Error response frame.
#[must_use]
pub fn error_response(request_id: &str, operation: &str, error: &str) -> Value {
    json!({
        "schema_version": RESPONSE_SCHEMA, "request_id": request_id, "operation": operation,
        "ok": false, "error": bounded_error(error),
    })
}

/// Success response frame.
#[must_use]
pub fn success_response(request: &Request, result: &Value) -> Value {
    json!({
        "schema_version": RESPONSE_SCHEMA, "request_id": request.request_id,
        "operation": request.operation.name(), "ok": true, "result": result,
    })
}

/// Terminal frame.
#[must_use]
pub fn terminal(task: &Value) -> Value {
    json!({"schema_version": TERMINAL_SCHEMA, "task": task})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(op: &str, payload: &Value) -> Value {
        json!({"schema_version": REQUEST_SCHEMA, "request_id": "r1", "operation": op, "payload": payload})
    }

    #[test]
    fn validates_frames() {
        assert_eq!(
            parse_request(&json!(1)).unwrap_err().error,
            "request frame must be an object"
        );
        let r = parse_request(
            &json!({"request_id": "x", "operation": "run", "schema_version": "bad", "payload": {}}),
        )
        .unwrap_err();
        assert_eq!(
            (
                r.request_id.as_str(),
                r.operation.as_str(),
                r.error.as_str()
            ),
            ("x", "run", "request schema_version mismatch")
        );
        let mut extra = frame("status", &json!({}));
        extra["extra"] = json!(1);
        assert_eq!(
            parse_request(&extra).unwrap_err().error,
            "request contains unknown key extra"
        );
        assert_eq!(
            parse_request(&frame("nope", &json!({}))).unwrap_err().error,
            "request.operation must be one of capabilities, run, status, logs, kill"
        );
        let long = json!({"schema_version": REQUEST_SCHEMA, "request_id": "r".repeat(201), "operation": "status", "payload": {}});
        assert!(
            parse_request(&long)
                .unwrap_err()
                .error
                .contains("at most 200")
        );
        assert_eq!(
            parse_request(&frame("capabilities", &json!({"a": 1})))
                .unwrap_err()
                .error,
            "capabilities.payload contains unknown key a"
        );
    }

    #[test]
    fn parses_operations() {
        let run = parse_request(&frame(
            "run",
            &json!({"name": "n", "command": "c", "isAgent": false, "notifyOnCompletion": true, "triggerOnCompletion": false, "timeoutSeconds": 5}),
        ));
        assert_eq!(
            run.map(|r| r.operation),
            Ok(Operation::Run {
                name: "n".into(),
                command: "c".into(),
                is_agent: false,
                timeout_seconds: Some(5),
                notify_on_completion: true,
                trigger_on_completion: false
            })
        );
        assert_eq!(
            parse_request(&frame("run", &json!({"name": "n", "command": "c", "isAgent": false, "notifyOnCompletion": true, "triggerOnCompletion": true, "timeoutSeconds": 0})))
                .unwrap_err()
                .error,
            "run.payload.timeoutSeconds must be a positive integer"
        );
        assert_eq!(
            parse_request(&frame("run", &json!({"name": "", "command": "c"})))
                .unwrap_err()
                .error,
            "run.payload.name must be a non-empty string"
        );
        assert_eq!(
            parse_request(&frame("logs", &json!({"taskId": "t", "tail": false})))
                .map(|r| r.operation),
            Ok(Operation::Logs {
                task_id: "t".into(),
                max_bytes: None,
                tail: Some(false)
            })
        );
        assert_eq!(
            parse_request(&frame("kill", &json!({}))).unwrap_err().error,
            "kill.payload.taskId must be a non-empty string"
        );
    }

    #[test]
    fn frames_and_bounds() {
        let e = error_response("r", "run", &format!("a  \n b{}", "x".repeat(300)));
        assert_eq!(e["ok"], false);
        assert_eq!(e["error"].as_str().map(|s| s.chars().count()), Some(240));
        assert!(e["error"].as_str().is_some_and(|s| s.starts_with("a b")));
        let req = Request {
            request_id: "r".into(),
            operation: Operation::Capabilities,
        };
        let s = success_response(&req, &capabilities());
        assert_eq!(s["operation"], "capabilities");
        assert_eq!(s["result"]["api_version"], 1);
        assert_eq!(
            terminal(&json!({"id": "t"}))["schema_version"],
            TERMINAL_SCHEMA
        );
    }
}
