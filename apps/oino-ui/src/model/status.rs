//! `StatusPanel` model (design.md §4.19): Goal, Progress, Run and Git sections derived from a
//! session. Until the sticky-topic feed exists (design.md §10 R3) the sources are the
//! session's own state: long-horizon / ralph statuses (Goal), the workflow's approved plan
//! (Progress — never arbitrary assistant lists, GA-10), running tool calls, subagents and
//! the footer process line (Run), and the footer frame's branch segment (Git).
//! [`StatusModel::ingest_topic`] accepts topic payloads for when the feed lands.

use oino_ai::JsonValue;

use crate::model::session::SessionModel;
use crate::model::timeline::{self, agent_line, plan_steps};
use crate::model::tools::ToolKind;
use crate::model::transcript::{RowKind, ToolStatus};

/// Fixed prefix of the workflow's approved-plan handoff user message
/// (`unipi.workflow` `approve_plan`: "Implement the approved plan below. …"). The plan is
/// the only Progress source; assistant replies are not (GA-10).
const APPROVED_PLAN_PREFIX: &str = "Implement the approved plan below.";

/// The plan body of an approved-plan handoff message.
#[must_use]
fn approved_plan(text: &str) -> Option<&str> {
    text.strip_prefix(APPROVED_PLAN_PREFIX).map(str::trim)
}

/// Kind of a Run row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    /// A shell command or background task.
    Task,
    /// A subagent.
    Agent,
    /// The fusion sidekick.
    Sidekick,
    /// A dream run.
    Dream,
}

/// One Run row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunItem {
    /// Stable id (tool call id or task id).
    pub id: String,
    /// Kind.
    pub kind: RunKind,
    /// Label (`cargo test --watch`, `explorer: map crate deps`).
    pub label: String,
    /// Elapsed text when known.
    pub elapsed: Option<String>,
}

/// One Progress step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Text.
    pub text: String,
    /// Done.
    pub done: bool,
}

/// Goal section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    /// Goal text.
    pub text: String,
    /// Paused.
    pub paused: bool,
    /// Extra state (`12m · 41k tok`).
    pub detail: Option<String>,
}

/// The panel content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusModel {
    /// Goal (long-horizon / ralph / swarm).
    pub goal: Option<Goal>,
    /// Plan steps.
    pub steps: Vec<Step>,
    /// Background tasks, subagents, sidekick.
    pub run: Vec<RunItem>,
    /// Git branch.
    pub branch: Option<String>,
    /// Changed files in the session.
    pub changed: usize,
}

impl StatusModel {
    /// Whether any section has data (the panel hides otherwise).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.goal.is_none() && self.steps.is_empty() && self.run.is_empty() && self.branch.is_none()
    }

    /// `done/total` of the plan.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (
            self.steps.iter().filter(|s| s.done).count(),
            self.steps.len(),
        )
    }

    /// Builds the panel from a session.
    #[allow(clippy::too_many_lines, reason = "one pass per section source")]
    #[must_use]
    pub fn from_session(session: &SessionModel) -> Self {
        let mut model = Self::default();
        for (key, status) in session.ui.statuses_keyed() {
            if key.starts_with("unipi.long-horizon/")
                || key.contains("ralph")
                || key.contains("goal")
            {
                let paused = status.text.contains("paused") || status.text.contains('⏸');
                model.goal = Some(Goal {
                    text: status.text.clone(),
                    paused,
                    detail: None,
                });
            }
        }
        for seg in session.ui.frame_segments() {
            let is_branch = seg.id.as_deref() == Some("branch")
                || seg.text.starts_with('⎇')
                || seg.icon.as_deref() == Some("branch");
            if is_branch {
                let b = seg
                    .text
                    .strip_prefix("branch:")
                    .unwrap_or(&seg.text)
                    .trim_start_matches(|c: char| !c.is_alphanumeric())
                    .trim();
                if !b.is_empty() {
                    model.branch = Some(b.to_owned());
                }
            }
        }
        let rows = session.transcript.rows();
        // Progress = the workflow's approved plan (the "Implement the approved plan below."
        // handoff), never an assistant reply: clarifying questions must not become todos
        // (GA-10). Steps advance with finished tool calls after the handoff; a checklist's
        // own `- [x]` marks win.
        if let Some(plan_at) = rows.iter().rposition(|r| match &r.kind {
            RowKind::User(u) => approved_plan(&u.text).is_some(),
            _ => false,
        }) {
            let plan = match &rows[plan_at].kind {
                RowKind::User(u) => approved_plan(&u.text).unwrap_or_default(),
                _ => "",
            };
            let steps = plan_steps(plan);
            if steps.len() >= 2 {
                let tools_done = rows[plan_at..]
                    .iter()
                    .filter(|r| {
                        matches!(&r.kind, RowKind::Tool(t) if matches!(t.status, ToolStatus::Done | ToolStatus::Error))
                    })
                    .count();
                let n = steps.len();
                model.steps = steps
                    .into_iter()
                    .enumerate()
                    .map(|(i, (text, done))| Step {
                        text,
                        done: done || i < tools_done.min(n.saturating_sub(1)),
                    })
                    .collect();
            }
        }
        for row in rows {
            let RowKind::Tool(tool) = &row.kind else {
                continue;
            };
            let running = matches!(tool.status, ToolStatus::Running | ToolStatus::Preparing);
            if !running {
                continue;
            }
            if let Some(agent) = agent_line(tool) {
                model.run.push(RunItem {
                    id: tool.call_id.clone(),
                    kind: if agent.sidekick {
                        RunKind::Sidekick
                    } else {
                        RunKind::Agent
                    },
                    label: if agent.task.is_empty() {
                        agent.name
                    } else {
                        format!("{}: {}", agent.name, agent.task)
                    },
                    elapsed: None,
                });
            } else if ToolKind::of(&tool.name) == ToolKind::Shell || tool.name.starts_with("bg_") {
                let line = timeline::tool_line(tool);
                let label = match line.subject {
                    timeline::Subject::Command(c) | timeline::Subject::Text(c) => c,
                    _ => tool.name.clone(),
                };
                model.run.push(RunItem {
                    id: tool.call_id.clone(),
                    kind: RunKind::Task,
                    label,
                    elapsed: None,
                });
            }
        }
        for (key, widget) in session.ui.widgets_keyed() {
            let process = key.starts_with("unipi.footer/") && key.contains("process")
                || key.starts_with("unipi.background-tasks/");
            if process {
                for (i, line) in widget.lines.iter().enumerate() {
                    model.run.push(RunItem {
                        id: format!("{key}#{i}"),
                        kind: RunKind::Task,
                        label: line.trim().to_owned(),
                        elapsed: None,
                    });
                }
            }
            if key.starts_with("unipi.dream/") {
                model.run.push(RunItem {
                    id: key.to_owned(),
                    kind: RunKind::Dream,
                    label: widget
                        .lines
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "Dream".into()),
                    elapsed: None,
                });
            }
        }
        model.changed = timeline::file_changes(rows).len();
        model
    }

    /// Applies a sticky-topic payload (R3 feed); unknown topics are ignored.
    pub fn ingest_topic(&mut self, topic: &str, payload: &JsonValue) {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        };
        match topic {
            "unipi:long-horizon:state" => {
                self.goal = s("goal").or_else(|| s("objective")).map(|text| Goal {
                    text,
                    paused: payload.get("paused").and_then(JsonValue::as_bool) == Some(true),
                    detail: s("detail"),
                });
            }
            "unipi:workflow:status" => {
                if let Some(steps) = payload.get("steps").and_then(JsonValue::as_array) {
                    self.steps = steps
                        .iter()
                        .filter_map(|st| {
                            Some(Step {
                                text: st.get("text")?.as_str()?.to_owned(),
                                done: st.get("done").and_then(JsonValue::as_bool) == Some(true),
                            })
                        })
                        .collect();
                }
            }
            "unipi:bg:tasks" => {
                self.run.retain(|r| r.kind != RunKind::Task);
                for task in payload
                    .get("tasks")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = task.get("id").and_then(JsonValue::as_str) {
                        self.run.push(RunItem {
                            id: id.to_owned(),
                            kind: RunKind::Task,
                            label: task
                                .get("command")
                                .or_else(|| task.get("name"))
                                .and_then(JsonValue::as_str)
                                .unwrap_or(id)
                                .to_owned(),
                            elapsed: task
                                .get("elapsed")
                                .and_then(JsonValue::as_str)
                                .map(str::to_owned),
                        });
                    }
                }
            }
            "unipi:footer:git" => self.branch = s("branch"),
            _ => {}
        }
    }

    /// Mini capsule parts: `(progress "2/3", running count, changed files)`.
    #[must_use]
    pub fn mini(&self) -> (Option<String>, usize, usize) {
        let (done, total) = self.progress();
        (
            (total > 0).then(|| format!("{done}/{total}")),
            self.run.len(),
            self.changed,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_core::events::ClientEvent;
    use serde_json::json;

    fn ev(v: serde_json::Value) -> ClientEvent {
        serde_json::from_value(v).unwrap()
    }

    fn assistant(text: &str) -> serde_json::Value {
        json!({"role":"assistant","content":[{"type":"text","text":text}],"api":"x","provider":"p","model":"m",
            "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop","timestamp":0})
    }

    #[test]
    fn derives_sections_from_the_session() {
        let mut s = SessionModel::new("s", "/w");
        assert!(StatusModel::from_session(&s).is_empty());
        s.apply_all(&[
            ev(json!({"type":"agent_start"})),
            ev(json!({"type":"message_start","message":{"role":"user","content":"go","timestamp":0}})),
            ev(json!({"type":"message_end","message":assistant("I will now:
1. Locate
2. Patch
3. Verify")})),
            ev(json!({"type":"message_start","message":{"role":"user","content":"Implement the approved plan below.\n\n# Fix\n## Implementation\n1. Locate\n2. Patch\n3. Verify","timestamp":0}})),
            ev(json!({"type":"tool_execution_start","toolCallId":"t1","toolName":"bash","args":{"command":"cargo test"}})),
            ev(json!({"type":"tool_execution_start","toolCallId":"t2","toolName":"run_subagent","args":{"agent":"explorer","task":"map deps"}})),
            ev(json!({"type":"extension_ui_request","id":"1","method":"setFrame","frameKey":"f","extensionId":"unipi.footer",
                "bottom":[{"id":"branch","text":"⎇ v1-alpha","priority":3,"align":"left"}]})),
            ev(json!({"type":"extension_ui_request","id":"2","method":"setStatus","statusKey":"goal","statusText":"Ship trailing commas","extensionId":"unipi.long-horizon"})),
        ]);
        let m = StatusModel::from_session(&s);
        assert_eq!(m.branch.as_deref(), Some("v1-alpha"));
        assert_eq!(m.goal.as_ref().unwrap().text, "Ship trailing commas");
        assert_eq!(m.progress(), (0, 3));
        assert_eq!(m.run.len(), 2);
        assert_eq!(m.run[1].kind, RunKind::Agent);
        assert_eq!(m.run[1].label, "explorer: map deps");
        assert_eq!(m.mini(), (Some("0/3".into()), 2, 0));
    }

    /// GA-10: the model's clarifying questions (or any assistant list) are not plan todos,
    /// and finished turns do not strike every step through.
    #[test]
    fn assistant_questions_are_not_plan_steps() {
        let mut s = SessionModel::new("s", "/w");
        s.apply_all(&[
            ev(json!({"type":"agent_start"})),
            ev(json!({"type":"message_start","message":{"role":"user","content":"/plan","timestamp":0}})),
            ev(json!({"type":"message_end","message":assistant("Three questions:
1. Which DB?
2. Which port?
3. Any budget?")})),
        ]);
        let m = StatusModel::from_session(&s);
        assert!(m.steps.is_empty(), "questions must not become Progress");
        s.apply_all(&[
            ev(json!({"type":"message_start","message":{"role":"user","content":"Implement the approved plan below.\n\n- [x] pick the DB\n- [ ] wire the port","timestamp":0}})),
            ev(json!({"type":"message_end","message":assistant("done")})),
        ]);
        let m = StatusModel::from_session(&s);
        assert_eq!(
            m.progress(),
            (1, 2),
            "checklist marks advance, the rest waits"
        );
    }

    #[test]
    fn topics_update_sections() {
        let mut m = StatusModel::default();
        m.ingest_topic(
            "unipi:long-horizon:state",
            &json!({"goal":"G","paused":true}),
        );
        m.ingest_topic(
            "unipi:workflow:status",
            &json!({"steps":[{"text":"a","done":true},{"text":"b"}]}),
        );
        m.ingest_topic(
            "unipi:bg:tasks",
            &json!({"tasks":[{"id":"t","command":"cargo watch","elapsed":"4m"}]}),
        );
        assert!(m.goal.as_ref().unwrap().paused);
        assert_eq!(m.progress(), (1, 2));
        assert_eq!(m.run[0].elapsed.as_deref(), Some("4m"));
        m.ingest_topic("nope", &json!({}));
    }
}
