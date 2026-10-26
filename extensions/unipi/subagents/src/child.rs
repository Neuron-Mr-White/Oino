//! Child-agent runs over the host `agents` interface (unipi core `child-agent/runtime.ts` +
//! `delivery.ts`). `unipi.fusion` compiles this same file for its sidekick (`#[path]` module).
//!
//! unipi drove a `pi --mode rpc` child over JSONL; Oino runs nested `AgentSession`s (D8) and
//! reads their `ClientEvent` stream. [`Progress`] folds that stream into unipi's live events
//! (text segments, tool calls with output), UI-only completed steps and the usage totals; a
//! handoff ends when the child settles with no wake-up background task open (3 s grace), like
//! unipi.

use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// unipi `MAX_EVENTS`.
pub const MAX_EVENTS: usize = 300;
/// unipi `MAX_TOOL_OUTPUT`.
pub const MAX_TOOL_OUTPUT: usize = 4000;
/// unipi `settleGraceMs` default.
pub const SETTLE_GRACE_MS: u64 = 3000;
/// unipi `BACKGROUND_DENY_NOTE` (host `UiPolicy::AutoDeny` note).
pub const BACKGROUND_DENY_NOTE: &str = "Permission denied for this tool: this agent is running in the background, where tools that would require approval are automatically denied. Do not retry variants of this call; finish what you can without it and say in your report what needs approval.";

/// unipi `SidekickUsage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Cache reads.
    pub cache_read: u64,
    /// Cache writes.
    pub cache_write: u64,
    /// USD.
    pub cost: f64,
}

impl Usage {
    fn add(&mut self, u: &Value) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        self.input += n("input");
        self.output += n("output");
        self.cache_read += n("cacheRead");
        self.cache_write += n("cacheWrite");
        self.cost += u
            .pointer("/cost/total")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
    }
}

/// unipi `SidekickEvent`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChildEvent {
    /// Streaming/closed text.
    Text {
        /// Text.
        text: String,
        /// Still streaming.
        open: bool,
    },
    /// A tool call.
    #[serde(rename_all = "camelCase")]
    Tool {
        /// Id.
        tool_call_id: String,
        /// Tool.
        name: String,
        /// Arguments.
        args: Option<Value>,
        /// Output (≤ 4000 chars, tail).
        output: String,
        /// Failed.
        is_error: bool,
        /// Finished.
        done: bool,
        /// Start (ms).
        started_at: i64,
        /// End (ms).
        ended_at: Option<i64>,
    },
}

/// unipi `SidekickStep` (UI-only `*-step` entries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Step {
    /// A finished tool.
    #[serde(rename_all = "camelCase")]
    Tool {
        /// Tool.
        name: String,
        /// Primary argument.
        arg: String,
        /// Output (last 40 lines, 300 chars each).
        output: String,
        /// Failed.
        is_error: bool,
        /// Duration.
        duration_ms: i64,
        /// Arguments.
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<Value>,
    },
    /// A text segment that was not the final report.
    Text {
        /// Text.
        text: String,
        /// Thinking.
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
    },
}

/// unipi `HandoffReport.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportStatus {
    /// Finished.
    Completed,
    /// Aborted on request.
    Aborted,
    /// Failed.
    Error,
    /// Interrupted.
    Interrupted,
}

/// unipi `HandoffReport`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Handoff/agent id.
    pub id: String,
    /// Status.
    pub status: ReportStatus,
    /// Final assistant text.
    pub text: String,
    /// Usage of this handoff.
    pub usage: Usage,
    /// Tool calls.
    pub tool_calls: u32,
    /// Duration.
    pub duration_ms: i64,
    /// Events.
    pub events: Vec<ChildEvent>,
    /// Error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn clip(value: &str, max: usize) -> String {
    if value.chars().count() > max {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        value.to_owned()
    }
}

/// unipi `primaryArg` (runtime flavour: `read/edit/write` path, else the first string).
#[must_use]
pub fn primary_arg(name: &str, args: Option<&Value>) -> String {
    let Some(args) = args.and_then(Value::as_object) else {
        return String::new();
    };
    let value = match name {
        "bash" => args.get("command"),
        "read" | "edit" | "write" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .or_else(|| args.get("filePath")),
        _ => args
            .get("pattern")
            .or_else(|| args.get("query"))
            .or_else(|| args.values().find(|v| v.is_string())),
    };
    value
        .and_then(Value::as_str)
        .map(|v| clip(v.split('\n').next().unwrap_or_default(), 100))
        .unwrap_or_default()
}

/// Text of a tool result (`AgentToolResult` or string), last 4000 chars.
#[must_use]
pub fn tool_output(result: &Value) -> String {
    let text = match result {
        Value::String(s) => s.clone(),
        _ => result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
    };
    let n = text.chars().count();
    text.chars()
        .skip(n.saturating_sub(MAX_TOOL_OUTPUT))
        .collect()
}

/// Live progress of one handoff.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Tool calls so far.
    pub tool_calls: u32,
    /// Last 6 tool summaries.
    pub recent_tools: Vec<String>,
    /// Last 400 chars of text.
    pub text_tail: String,
    /// Events (≤ 300).
    pub events: Vec<ChildEvent>,
    /// Usage.
    pub usage: Usage,
    /// Wake-up `bg_run` tasks still open.
    pub open_bg_tasks: u32,
    /// The child settled (agent end).
    pub settled: bool,
    /// Last assistant error (`stopReason: "error"`).
    pub pending_error: Option<String>,
    buffered_text: Option<(String, String)>,
}

impl Progress {
    fn push(&mut self, e: ChildEvent) {
        self.events.push(e);
        if self.events.len() > MAX_EVENTS {
            let drop = self.events.len() - MAX_EVENTS;
            self.events.drain(..drop);
        }
    }

    fn close_open_text(&mut self) {
        if let Some(ChildEvent::Text { open, .. }) = self.events.last_mut() {
            *open = false;
        }
    }

    fn flush_text_step(&mut self) -> Option<Step> {
        let (text, thinking) = self.buffered_text.take()?;
        (!text.trim().is_empty()).then(|| Step::Text {
            text,
            thinking: (!thinking.is_empty()).then_some(thinking),
        })
    }

    fn tool_mut(&mut self, id: &str) -> Option<&mut ChildEvent> {
        self.events
            .iter_mut()
            .rev()
            .find(|e| matches!(e, ChildEvent::Tool { tool_call_id, .. } if tool_call_id == id))
    }

    /// Folds one child `ClientEvent`; returns completed steps.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per child event type, as unipi"
    )]
    pub fn apply(&mut self, ev: &Value, now: i64) -> Vec<Step> {
        let mut steps = Vec::new();
        let ty = ev.get("type").and_then(Value::as_str).unwrap_or_default();
        let s = |k: &str| {
            ev.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match ty {
            "tool_execution_start" => {
                self.close_open_text();
                steps.extend(self.flush_text_step());
                self.tool_calls += 1;
                let args = ev.get("args").filter(|a| a.is_object()).cloned();
                let name = ev
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_owned();
                if name == "bg_run"
                    && args.as_ref().is_none_or(|a| {
                        a.get("notifyOnCompletion") != Some(&json!(false))
                            && a.get("triggerOnCompletion") != Some(&json!(false))
                    })
                {
                    self.open_bg_tasks += 1;
                }
                let args_text = args
                    .as_ref()
                    .map(|a| {
                        a.to_string()
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let summary: String = format!("{name}({args_text})").chars().take(40).collect();
                self.recent_tools.push(summary);
                let n = self.recent_tools.len();
                if n > 6 {
                    self.recent_tools.drain(..n - 6);
                }
                self.push(ChildEvent::Tool {
                    tool_call_id: s("toolCallId"),
                    name,
                    args,
                    output: String::new(),
                    is_error: false,
                    done: false,
                    started_at: now,
                    ended_at: None,
                });
            }
            "tool_execution_update" => {
                let id = s("toolCallId");
                if let Some(partial) = ev.get("partialResult")
                    && let Some(ChildEvent::Tool {
                        output,
                        done: false,
                        ..
                    }) = self.tool_mut(&id)
                {
                    *output = tool_output(partial);
                }
            }
            "tool_execution_end" => {
                let id = s("toolCallId");
                let is_err = ev.get("isError").and_then(Value::as_bool).unwrap_or(false);
                let result = ev.get("result").cloned().unwrap_or(Value::Null);
                let mut finished_bg = false;
                if let Some(ChildEvent::Tool {
                    name,
                    args,
                    output,
                    is_error,
                    done,
                    started_at,
                    ended_at,
                    ..
                }) = self.tool_mut(&id)
                {
                    *done = true;
                    *ended_at = Some(now);
                    *is_error = is_err;
                    *output = tool_output(&result);
                    finished_bg = name == "bg_run" && is_err;
                    let cap = |l: &str| clip(l, 300);
                    let lines: Vec<&str> = output.split('\n').collect();
                    let tail = lines[lines.len().saturating_sub(40)..]
                        .iter()
                        .map(|l| cap(l))
                        .collect::<Vec<_>>()
                        .join("\n");
                    steps.push(Step::Tool {
                        arg: primary_arg(name, args.as_ref()),
                        name: name.clone(),
                        args: args.clone(),
                        output: tail,
                        is_error: is_err,
                        duration_ms: now - *started_at,
                    });
                }
                if finished_bg {
                    self.open_bg_tasks = self.open_bg_tasks.saturating_sub(1);
                }
            }
            "message_update" => {
                let se = ev.get("assistantMessageEvent").unwrap_or(ev);
                match se.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let delta = se
                            .get("delta")
                            .or_else(|| se.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        self.text_tail.push_str(delta);
                        let n = self.text_tail.chars().count();
                        if n > 400 {
                            self.text_tail = self.text_tail.chars().skip(n - 400).collect();
                        }
                        match self.events.last_mut() {
                            Some(ChildEvent::Text { text, open: true }) => text.push_str(delta),
                            _ => self.push(ChildEvent::Text {
                                text: delta.to_owned(),
                                open: true,
                            }),
                        }
                        self.buffered_text
                            .get_or_insert_with(Default::default)
                            .0
                            .push_str(delta);
                    }
                    Some("thinking_delta") => {
                        let delta = se.get("delta").and_then(Value::as_str).unwrap_or_default();
                        self.buffered_text
                            .get_or_insert_with(Default::default)
                            .1
                            .push_str(delta);
                    }
                    _ => {}
                }
            }
            "message_end" => {
                let msg = ev.get("message").unwrap_or(&Value::Null);
                match msg.get("role").and_then(Value::as_str) {
                    Some("custom")
                        if msg.get("customType").and_then(Value::as_str)
                            == Some("background-task-notification") =>
                    {
                        self.open_bg_tasks = self.open_bg_tasks.saturating_sub(1);
                    }
                    Some("assistant") => {
                        self.close_open_text();
                        if msg.get("stopReason").and_then(Value::as_str) == Some("error") {
                            self.pending_error = msg
                                .get("errorMessage")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned);
                        }
                        if let Some(u) = msg.get("usage") {
                            self.usage.add(u);
                        }
                    }
                    _ => {}
                }
            }
            "message_start" => steps.extend(self.flush_text_step()),
            "agent_start" => self.settled = false,
            "agent_end" | "agent_settled" => self.settled = true,
            _ => {}
        }
        steps
    }

    /// The handoff may finish: settled and no wake-up background task open.
    #[must_use]
    pub fn finishable(&self) -> bool {
        self.settled && self.open_bg_tasks == 0
    }

    /// Builds the final report (the buffered last text segment is the report, never a step).
    #[must_use]
    pub fn report(
        &mut self,
        id: &str,
        aborted: bool,
        text: Option<String>,
        error: Option<String>,
        duration_ms: i64,
    ) -> Report {
        self.buffered_text = None;
        let error = error.or_else(|| self.pending_error.clone());
        let status = if aborted {
            ReportStatus::Aborted
        } else if error.is_some() {
            ReportStatus::Error
        } else {
            ReportStatus::Completed
        };
        Report {
            id: id.to_owned(),
            status,
            text: text.unwrap_or_else(|| self.text_tail.clone()),
            usage: self.usage,
            tool_calls: self.tool_calls,
            duration_ms,
            events: self.events.clone(),
            error,
        }
    }
}

/// A one-shot completion cell many waiters can await.
#[derive(Default)]
pub struct Done<T: Clone> {
    value: RefCell<Option<T>>,
    waiters: RefCell<Vec<oneshot::Sender<()>>>,
}

impl<T: Clone> Done<T> {
    /// A new cell.
    #[must_use]
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            value: RefCell::new(None),
            waiters: RefCell::new(Vec::new()),
        })
    }

    /// Sets the value (first set wins) and wakes the waiters.
    pub fn set(&self, v: T) {
        if self.value.borrow().is_some() {
            return;
        }
        *self.value.borrow_mut() = Some(v);
        for tx in self.waiters.borrow_mut().drain(..) {
            let _ = tx.send(());
        }
    }

    /// The value when set.
    #[must_use]
    pub fn get(&self) -> Option<T> {
        self.value.borrow().clone()
    }

    /// Waits for the value.
    pub async fn wait(&self) -> T {
        loop {
            if let Some(v) = self.get() {
                return v;
            }
            let (tx, rx) = oneshot::channel();
            self.waiters.borrow_mut().push(tx);
            let _ = rx.await;
        }
    }
}

/// unipi `createCompletionDelivery`: a completion is delivered only when it lands while no
/// waiter is attached, once per id.
#[derive(Default)]
pub struct Delivery {
    waiting: RefCell<std::collections::HashSet<String>>,
    armed: RefCell<std::collections::HashSet<String>>,
    delivered: RefCell<std::collections::HashSet<String>>,
}

impl Delivery {
    /// A waiter attached.
    pub fn attach(&self, id: &str) {
        self.waiting.borrow_mut().insert(id.to_owned());
    }

    /// The waiter gave up; returns `true` when the caller should arm a delivery continuation
    /// (first time only).
    pub fn detach(&self, id: &str) -> bool {
        self.waiting.borrow_mut().remove(id);
        self.armed.borrow_mut().insert(id.to_owned())
    }

    /// The report landed: deliver now?
    pub fn should_deliver(&self, id: &str) -> bool {
        if self.waiting.borrow().contains(id) || self.delivered.borrow().contains(id) {
            return false;
        }
        self.delivered.borrow_mut().insert(id.to_owned())
    }

    /// A waiter consumed the report.
    pub fn consume(&self, id: &str) {
        self.waiting.borrow_mut().remove(id);
        self.delivered.borrow_mut().insert(id.to_owned());
    }
}

/// The host `AgentSpec` JSON (runtime §12, camelCase).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    /// Label (prompt prefix in forwarded dialogs).
    pub label: String,
    /// `provider/id`.
    pub model: Option<String>,
    /// Thinking level.
    pub thinking: Option<String>,
    /// Appended system prompt.
    pub append_system_prompt: Option<String>,
    /// Tool allowlist (`None` = all tools minus `deny`).
    pub allow: Option<Vec<String>>,
    /// Denied tools.
    pub deny: Vec<String>,
    /// Resume from this session file.
    pub seed_session_file: Option<String>,
    /// Persist child sessions in this host directory.
    pub session_dir: Option<String>,
    /// Child role (`subagent`, `fusion`).
    pub child_role: String,
    /// Forward dialogs to the parent UI (else auto-deny with [`BACKGROUND_DENY_NOTE`]).
    pub forward_ui: bool,
    /// Depth cap.
    pub max_depth: u8,
}

/// pi built-in tool names (everything else is an extension tool).
pub const BUILTIN_TOOLS: &[&str] = &["read", "bash", "edit", "write", "grep", "find", "ls"];

impl Spec {
    /// Serialises to the host `AgentSpec` JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let (builtin, extension): (Vec<String>, Vec<String>) = match &self.allow {
            Some(list) => list
                .iter()
                .cloned()
                .partition(|t| BUILTIN_TOOLS.contains(&t.as_str())),
            None => (
                BUILTIN_TOOLS.iter().map(|s| (*s).to_owned()).collect(),
                vec!["*".to_owned()],
            ),
        };
        let mut spec = json!({
            "label": self.label,
            "tools": {"builtin": builtin, "extensionTools": extension, "deny": self.deny},
            "extensions": "inherit",
            "seed": self.seed_session_file.as_ref().map_or_else(|| json!("none"), |f| json!({"sessionFile": f})),
            "persistence": self.session_dir.as_ref().map_or_else(|| json!("memory"), |d| json!({"file": {"sessionDir": d}})),
            "childRole": self.child_role,
            "ui": if self.forward_ui { json!("forward") } else { json!({"autoDeny": {"note": BACKGROUND_DENY_NOTE}}) },
            "maxDepth": self.max_depth,
        });
        if let Some(m) = &self.model {
            spec["model"] = json!(m);
        }
        if let Some(t) = &self.thinking {
            spec["thinking"] = json!(t);
        }
        if let Some(p) = self.append_system_prompt.as_ref().filter(|p| !p.is_empty()) {
            spec["systemPrompt"] = json!({"append": p});
        }
        spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_events_into_progress_and_steps() {
        let mut p = Progress::default();
        let delta = |d: &str| json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": d}});
        assert_eq!(
            p.apply(&json!({"type": "agent_start"}), 0),
            Vec::<Step>::new()
        );
        p.apply(&delta("Looking "), 1);
        p.apply(&delta("around"), 2);
        let steps = p.apply(&json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "read", "args": {"path": "/a\nb"}}), 3);
        assert_eq!(
            steps,
            vec![Step::Text {
                text: "Looking around".into(),
                thinking: None
            }]
        );
        p.apply(&json!({"type": "tool_execution_update", "toolCallId": "c1", "partialResult": {"content": [{"type": "text", "text": "par"}]}}), 4);
        let steps = p.apply(&json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "read", "isError": false, "result": {"content": [{"type": "text", "text": "full"}]}}), 10);
        assert!(
            matches!(&steps[0], Step::Tool { name, arg, output, duration_ms: 7, .. } if name == "read" && arg == "/a" && output == "full")
        );
        assert_eq!(p.tool_calls, 1);
        assert_eq!(p.recent_tools, ["read({\"path\":\"/a\\nb\"})"]);
        p.apply(&delta("Final report"), 11);
        p.apply(&json!({"type": "message_end", "message": {"role": "assistant", "usage": {"input": 10, "output": 5, "cost": {"total": 0.5}}}}), 12);
        p.apply(&json!({"type": "agent_end"}), 13);
        assert!(p.finishable());
        let r = p.report("abcd", false, Some("Final report".into()), None, 13);
        assert_eq!(r.status, ReportStatus::Completed);
        assert_eq!((r.usage.input, r.usage.output), (10, 5));
        assert!((r.usage.cost - 0.5).abs() < 1e-9);
        assert_eq!(r.events.len(), 3);
    }

    #[test]
    fn background_tasks_hold_the_settle() {
        let mut p = Progress::default();
        p.apply(&json!({"type": "tool_execution_start", "toolCallId": "b", "toolName": "bg_run", "args": {"command": "x"}}), 0);
        p.apply(&json!({"type": "agent_end"}), 1);
        assert!(!p.finishable());
        p.apply(&json!({"type": "message_end", "message": {"role": "custom", "customType": "background-task-notification"}}), 2);
        assert!(p.finishable());
        let mut q = Progress::default();
        q.apply(&json!({"type": "tool_execution_start", "toolCallId": "b", "toolName": "bg_run", "args": {"triggerOnCompletion": false}}), 0);
        assert_eq!(q.open_bg_tasks, 0);
        q.apply(&json!({"type": "message_end", "message": {"role": "assistant", "stopReason": "error", "errorMessage": "boom"}}), 1);
        assert_eq!(
            q.report("x", false, None, None, 1).status,
            ReportStatus::Error
        );
        assert_eq!(
            q.report("x", true, None, None, 1).status,
            ReportStatus::Aborted
        );
    }

    #[test]
    fn delivery_is_exactly_once() {
        let d = Delivery::default();
        d.attach("a");
        assert!(!d.should_deliver("a"), "waiter attached");
        assert!(d.detach("a"));
        assert!(!d.detach("a"), "armed once");
        assert!(d.should_deliver("a"));
        assert!(!d.should_deliver("a"));
        d.consume("b");
        assert!(!d.should_deliver("b"));
    }

    #[test]
    fn spec_json() {
        let s = Spec {
            label: "Subagent \"t\"".into(),
            model: Some("a/b".into()),
            allow: Some(vec!["read".into(), "web_search".into()]),
            deny: vec![],
            child_role: "subagent".into(),
            max_depth: 1,
            ..Spec::default()
        };
        let v = s.to_json();
        assert_eq!(v["tools"]["builtin"], json!(["read"]));
        assert_eq!(v["tools"]["extensionTools"], json!(["web_search"]));
        assert_eq!(v["ui"]["autoDeny"]["note"], BACKGROUND_DENY_NOTE);
        assert_eq!(v["seed"], "none");
        assert_eq!(v["persistence"], "memory");
        let g = Spec {
            deny: vec!["run_subagent".into()],
            forward_ui: true,
            seed_session_file: Some("/s.jsonl".into()),
            session_dir: Some("/d".into()),
            ..Spec::default()
        }
        .to_json();
        assert_eq!(g["tools"]["extensionTools"], json!(["*"]));
        assert_eq!(g["ui"], "forward");
        assert_eq!(g["seed"]["sessionFile"], "/s.jsonl");
        assert_eq!(g["persistence"]["file"]["sessionDir"], "/d");
        assert_eq!(primary_arg("grep", Some(&json!({"pattern": "fo"}))), "fo");
        assert_eq!(tool_output(&json!("x".repeat(5000))).len(), MAX_TOOL_OUTPUT);
        let done: Rc<Done<u8>> = Done::new();
        done.set(1);
        done.set(2);
        assert_eq!(done.get(), Some(1));
    }
}
