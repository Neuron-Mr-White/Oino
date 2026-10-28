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

/// Epoch milliseconds now (elapsed text of running background tasks).
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

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
    /// Stable id (tool call id, task id, agent id or topic key).
    pub id: String,
    /// Kind.
    pub kind: RunKind,
    /// Label (`cargo test --watch`, `explorer: map crate deps`).
    pub label: String,
    /// Elapsed text when known.
    pub elapsed: Option<String>,
    /// Child session behind the row (`_oino/agents`): Enter / click opens its Agent tab.
    pub agent_id: Option<String>,
}

impl RunItem {
    /// A row that opens a child session's Agent tab.
    fn agent(id: &str, kind: RunKind, label: String) -> Self {
        Self {
            id: id.to_owned(),
            kind,
            label,
            elapsed: None,
            agent_id: Some(id.to_owned()),
        }
    }
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
                    agent_id: agent.child.clone(),
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
                    agent_id: None,
                });
            }
        }
        // Child sessions from the `_oino/agents` feed that have no tool row of their own:
        // `btw`, a dream run, anything else an extension spawned (GA-15 / GA-44). Helper
        // children (the auto-rename title run) stay out of Run.
        let child_ids: Vec<Option<String>> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Tool(tool) => agent_line(tool).map(|a| a.child),
                _ => None,
            })
            .collect();
        for info in &session.agents {
            if info.parent_id.is_some() || info.state == oino_core::agents::AgentRunState::Closed {
                continue;
            }
            if info.owner == "unipi.utility" || child_ids.contains(&Some(info.id.clone())) {
                continue;
            }
            // A subagent / sidekick row without a child id still covers its agent.
            let covered = child_ids.contains(&None)
                && (info.owner.contains("subagent") || info.owner.contains("fusion"));
            if covered {
                continue;
            }
            let label = info.label.trim();
            let label = if label.is_empty() {
                info.owner.clone()
            } else {
                label.to_owned()
            };
            let kind = if label.contains("dream") || info.owner.contains("dream") {
                RunKind::Dream
            } else if label.contains("sidekick") || info.owner.contains("fusion") {
                RunKind::Sidekick
            } else {
                RunKind::Agent
            };
            model.run.push(RunItem::agent(&info.id, kind, label));
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
                        agent_id: None,
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
                    agent_id: None,
                });
            }
        }
        model.changed = timeline::file_changes(rows).len();
        model
    }

    /// Applies a sticky-topic payload (R3 feed); unknown topics are ignored.
    #[allow(clippy::too_many_lines, reason = "one arm per sticky topic")]
    pub fn ingest_topic(&mut self, topic: &str, payload: &JsonValue) {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        };
        match topic {
            // `{mode, paused, owner, lastStop}` — the goal text is in the progress topic.
            "unipi:long-horizon:state" => {
                let mode = s("mode").unwrap_or_default();
                if mode == "none" {
                    self.goal = None;
                } else if let Some(goal) = &mut self.goal {
                    goal.paused = s("paused").is_some();
                } else {
                    let paused = s("paused").is_some();
                    self.goal = Some(Goal {
                        text: s("owner").unwrap_or_else(|| mode.clone()),
                        paused,
                        detail: None,
                    });
                }
            }
            // `{mode, goal:{objective,status,turn,maxTurns}, ralph:{…}, graph:{…}, todos}`.
            "unipi:long-horizon:progress" => {
                let goal = payload.get("goal").filter(|g| !g.is_null());
                match goal {
                    Some(goal) => {
                        let objective = goal
                            .get("objective")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let turn = goal.get("turn").and_then(JsonValue::as_u64);
                        let max = goal.get("maxTurns").and_then(JsonValue::as_u64);
                        let status = goal
                            .get("status")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default();
                        let detail = match (turn, max) {
                            (Some(t), Some(m)) => Some(format!("turn {t}/{m}")),
                            (Some(t), None) => Some(format!("turn {t}")),
                            _ => (!status.is_empty()).then(|| status.to_owned()),
                        };
                        if !objective.is_empty() {
                            let paused = self.goal.as_ref().is_some_and(|g| g.paused);
                            self.goal = Some(Goal {
                                text: objective,
                                paused,
                                detail,
                            });
                        }
                    }
                    None => {
                        if payload.get("mode").and_then(JsonValue::as_str) == Some("none") {
                            self.goal = None;
                        }
                    }
                }
            }
            // `{planMode, permissionMode}`: the core plan steps come from the approved-plan
            // handoff (GA-10); a topic that carries explicit steps wins.
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
                self.run
                    .retain(|r| r.kind != RunKind::Task || r.id == "kanboard");
                for task in crate::components::side_pane::bg_tasks(payload) {
                    let elapsed = (task.status == "running")
                        .then(|| timeline::duration(task.elapsed_ms(now_ms())));
                    self.run.push(RunItem {
                        id: task.id,
                        kind: RunKind::Task,
                        label: task.command,
                        elapsed,
                        agent_id: None,
                    });
                }
            }
            // `{leadName, sidekickName, busy, …}`: a handoff in flight is a Run row.
            "unipi:fusion:status" => {
                self.run
                    .retain(|r| r.kind != RunKind::Sidekick || r.agent_id.is_some());
                if payload.get("busy").and_then(JsonValue::as_bool) == Some(true) {
                    let name = s("sidekickName").unwrap_or_else(|| "Sidekick".to_owned());
                    self.run.push(RunItem {
                        id: "fusion-sidekick".to_owned(),
                        kind: RunKind::Sidekick,
                        label: name,
                        elapsed: None,
                        agent_id: None,
                    });
                }
            }
            // `{claims:[taskId], autowork}`: claimed kanboard tasks show in Run.
            "unipi:kanboard:status" => {
                self.run.retain(|r| r.id != "kanboard");
                let claims = payload
                    .get("claims")
                    .and_then(JsonValue::as_array)
                    .map_or(0, Vec::len);
                if claims > 0 {
                    self.run.push(RunItem {
                        id: "kanboard".to_owned(),
                        kind: RunKind::Task,
                        label: format!("Kanboard · {claims} claimed"),
                        elapsed: None,
                        agent_id: None,
                    });
                }
            }
            // Core git topic: `{branch, head, dirty, changed, ahead, behind}`.
            oino_core::topics::GIT_TOPIC => {
                self.branch = s("branch");
                if let Some(changed) = payload.get("changed").and_then(JsonValue::as_u64) {
                    self.changed = usize::try_from(changed).unwrap_or(usize::MAX);
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

    /// The real unipi shapes (unipi-kit `topics`): the goal text lives in
    /// `long-horizon:progress`, the pause flag in `long-horizon:state`, tasks in
    /// `bg:tasks`, and `oino:git` carries the branch and the changed count.
    #[test]
    fn topics_update_sections() {
        let mut m = StatusModel::default();
        m.ingest_topic(
            "unipi:long-horizon:state",
            &json!({"mode": "ralph", "paused": "owner"}),
        );
        assert!(m.goal.as_ref().unwrap().paused);
        m.ingest_topic(
            "unipi:long-horizon:progress",
            &json!({"goal": {"objective": "G", "status": "active", "turn": 2, "maxTurns": 5}}),
        );
        let goal = m.goal.clone().unwrap();
        assert_eq!(goal.text, "G");
        assert!(goal.paused, "the state topic's pause flag survives");
        assert_eq!(goal.detail.as_deref(), Some("turn 2/5"));
        m.ingest_topic(
            "unipi:workflow:status",
            &json!({"planMode": false, "permissionMode": "auto",
                "steps": [{"text": "a", "done": true}, {"text": "b"}]}),
        );
        assert_eq!(m.progress(), (1, 2));
        m.ingest_topic(
            "unipi:bg:tasks",
            &json!({"tasks": [{"id": "t", "command": "cargo watch", "status": "running",
                "startTime": 0}]}),
        );
        assert!(m.run[0].elapsed.is_some(), "a running task shows elapsed");
        m.ingest_topic(
            "unipi:fusion:status",
            &json!({"leadName": "Lead", "sidekickName": "Haiku", "busy": true}),
        );
        assert!(m.run.iter().any(|r| r.kind == RunKind::Sidekick));
        m.ingest_topic(
            "unipi:kanboard:status",
            &json!({"claims": ["OIN-1"], "autowork": true}),
        );
        assert!(m.run.iter().any(|r| r.id == "kanboard"));
        m.ingest_topic(
            oino_core::topics::GIT_TOPIC,
            &json!({"branch": "v1-alpha", "changed": 3}),
        );
        assert_eq!(m.branch.as_deref(), Some("v1-alpha"));
        assert_eq!(m.changed, 3);
        m.ingest_topic("unipi:long-horizon:state", &json!({"mode": "none"}));
        assert!(m.goal.is_none(), "mode none clears the goal");
        m.ingest_topic("nope", &json!({}));
    }

    /// A child session without a tool row (`btw`) shows in Run and opens its Agent tab.
    #[test]
    fn btw_child_shows_in_run() {
        let mut s = SessionModel::new("s", "/w");
        s.agents = vec![btw("b1", "btw", "unipi.btw")];
        let m = StatusModel::from_session(&s);
        let row = m.run.iter().find(|r| r.agent_id.as_deref() == Some("b1"));
        assert!(row.is_some(), "the btw child is a Run row: {:?}", m.run);
        // The auto-rename helper child stays out of Run.
        s.agents.push(btw("h1", "session name", "unipi.utility"));
        let m = StatusModel::from_session(&s);
        assert!(!m.run.iter().any(|r| r.agent_id.as_deref() == Some("h1")));
    }

    fn btw(id: &str, label: &str, owner: &str) -> oino_core::agents::AgentInfo {
        oino_core::agents::AgentInfo {
            id: id.into(),
            parent_id: None,
            label: label.into(),
            owner: owner.into(),
            role: None,
            state: oino_core::agents::AgentRunState::Running,
            depth: 1,
            session_file: None,
        }
    }
}
