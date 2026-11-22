//! The turn arbiter's kanboard nudge provider (unipi `monitor.ts`).
//!
//! When the lead settles with claims of its own still In Progress, the monitor proposes ONE
//! continuation nudge (priority 50, claims; priority 40, autowork-next). The host arbiter
//! delivers at most one nudge per settle, so the monitor never fights the long-horizon owner
//! (priority 100) or a pending event. Lead only — children never propose.
//!
//! unipi's arbiter handed providers a `SettleInfo`; the host arbiter does not, so the
//! extension builds [`SettleInfo`] itself from `agent_start`/`tool_call`/`agent_end`.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::reminders::{ANTI_POISONING_SUFFIX, task_ids_in};
use crate::shapes::Task;

/// Custom type of the claims nudge.
pub const CLAIMS_NUDGE_CUSTOM_TYPE: &str = "unipi:kanboard-continue";
/// Custom type of the autowork nudge.
pub const AUTOWORK_NUDGE_CUSTOM_TYPE: &str = "unipi:kanboard-next";
/// Claims nudge priority.
pub const CLAIMS_PRIORITY: i64 = 50;
/// Autowork nudge priority.
pub const AUTOWORK_PRIORITY: i64 = 40;
/// Hard cap of continuation nudges per task.
pub const MAX_NUDGES_PER_TASK: u32 = 5;
/// Consecutive nudged runs with zero tool calls before the monitor gives up.
pub const STALL_LIMIT: u32 = 2;
/// Offers of the SAME ready task before autowork gives up on it.
pub const MAX_OFFERS_PER_TASK: u32 = 3;

/// What the run that is settling did (unipi `SettleInfo`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettleInfo {
    /// Text of the last assistant message.
    pub last_assistant_text: String,
    /// The last assistant message called tools.
    pub last_assistant_had_tool_calls: bool,
    /// Tool calls made during this run.
    pub tool_calls_this_run: u32,
}

/// The long-horizon owner as the sticky `unipi:long-horizon:state` reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnerStatus {
    /// Owner status (`active`, `parked`), when an owner exists.
    pub owner_status: Option<String>,
    /// Last stop `(kind, at_ms)`.
    pub last_stop: Option<(String, i64)>,
}

/// A notice for the user (never LLM context).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Text.
    pub text: String,
    /// `info` or `warning`.
    pub level: &'static str,
}

/// What a nudge does when the arbiter delivers it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Delivered {
    Claim { id: String, n: u32 },
    Offer { id: String, offers: u32 },
}

/// A proposed nudge (the host's `{id, priority, customType, content, display, details}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// Nudge id (passed back by `nudge-delivered`).
    pub id: String,
    /// Priority.
    pub priority: i64,
    /// Custom type.
    pub custom_type: &'static str,
    /// Content.
    pub content: String,
    /// Provenance title.
    pub title: &'static str,
    /// Provenance synopsis (the task id).
    pub synopsis: String,
    /// Warning severity.
    pub warning: bool,
}

impl Proposal {
    /// The host nudge JSON.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut provenance = json!({
            "source": "Kanboard",
            "title": self.title,
            "synopsis": self.synopsis,
            "delivery": "boundary",
        });
        if self.warning {
            provenance["severity"] = json!("warning");
        }
        json!({
            "id": self.id,
            "priority": self.priority,
            "customType": self.custom_type,
            "content": self.content,
            "display": true,
            "details": {"oinoProvenance": provenance},
        })
    }
}

/// The outcome of one proposal round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The nudge, if any.
    pub proposal: Option<Proposal>,
    /// Notices to queue.
    pub notices: Vec<Notice>,
    /// The monitor turned autowork off itself (done / stalled).
    pub autowork_off: bool,
}

/// Monitor state of one session.
#[allow(
    clippy::struct_excessive_bools,
    reason = "unipi's monitor flags, one per closure variable"
)]
#[derive(Clone, Debug, Default)]
pub struct Monitor {
    armed: bool,
    autowork: bool,
    run_started_at: i64,
    last_run_was_nudge: bool,
    nudge_delivered: bool,
    no_progress_runs: u32,
    nudges_per_task: HashMap<String, u32>,
    offers_per_task: HashMap<String, u32>,
    notified: HashSet<String>,
    pending: HashMap<String, Delivered>,
    seq: u64,
}

/// Last assistant message aborted or errored (`stopReason`).
#[must_use]
pub fn run_failed(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        .and_then(|m| m.get("stopReason").and_then(Value::as_str))
        .is_some_and(|r| r == "aborted" || r == "error")
}

/// Text and tool-call presence of the last assistant message.
#[must_use]
pub fn last_assistant(messages: &[Value]) -> (String, bool) {
    let Some(last) = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
    else {
        return (String::new(), false);
    };
    let blocks = last
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let text = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let tools = blocks
        .iter()
        .any(|b| b.get("type").and_then(Value::as_str) == Some("toolCall"));
    (text, tools)
}

/// The final non-empty paragraph of a text, trimmed.
fn final_paragraph(text: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs.pop().unwrap_or_default().trim().to_owned()
}

/// Our own runtime texts never count as user prompts naming claimed ids.
fn is_own_runtime_text(text: &str) -> bool {
    text.contains(ANTI_POISONING_SUFFIX)
        || text.contains(CLAIMS_NUDGE_CUSTOM_TYPE)
        || text.contains(AUTOWORK_NUDGE_CUSTOM_TYPE)
}

impl Monitor {
    fn notify_once(&mut self, out: &mut Outcome, key: String, text: String, level: &'static str) {
        if self.notified.insert(key) {
            out.notices.push(Notice { text, level });
        }
    }

    /// `agent_start`: the run that is about to start; stamps the run start.
    pub fn on_agent_start(&mut self, now_ms: i64) {
        self.last_run_was_nudge = self.nudge_delivered;
        self.nudge_delivered = false;
        self.run_started_at = now_ms;
    }

    /// The task ids a user prompt names, when it may re-arm (`None` = ignore the prompt).
    #[must_use]
    pub fn prompt_ids(text: &str) -> Option<Vec<String>> {
        if is_own_runtime_text(text) {
            return None;
        }
        let ids = task_ids_in(text);
        (!ids.is_empty()).then_some(ids)
    }

    /// A user prompt named `named`: re-arm when one of them is our claim.
    pub fn on_user_prompt(&mut self, named: &[String], tasks: &[Task], session: &str) {
        if tasks
            .iter()
            .any(|t| t.claimed_by(session) && named.contains(&t.id))
        {
            self.armed = true;
            self.notified.clear();
        }
    }

    /// `agent_end`: an aborted or errored run disarms (kanboard never re-nudges after Esc).
    pub fn on_agent_end(&mut self, messages: &[Value]) {
        if run_failed(messages) {
            self.armed = false;
        }
    }

    /// Arms the monitor (from `-do`, autowork start, a successful lead `start`, an owner
    /// resume).
    pub fn arm(&mut self) {
        self.armed = true;
        self.notified.clear();
    }

    /// Disarms.
    pub fn disarm(&mut self) {
        self.armed = false;
    }

    /// Autowork on/off.
    pub fn set_autowork(&mut self, on: bool) {
        self.autowork = on;
    }

    /// Armed (tests / status).
    #[must_use]
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Autowork on.
    #[must_use]
    pub fn autowork(&self) -> bool {
        self.autowork
    }

    /// Needs the ready list for this round (autowork with no claims to nudge).
    #[must_use]
    pub fn wants_ready(&self) -> bool {
        self.armed && self.autowork
    }

    fn next_id(&mut self) -> String {
        self.seq += 1;
        format!("kanboard-{}", self.seq)
    }

    fn update_stall(&mut self, info: &SettleInfo) {
        if self.last_run_was_nudge && info.tool_calls_this_run == 0 {
            self.no_progress_runs += 1;
        } else if info.tool_calls_this_run > 0 {
            self.no_progress_runs = 0;
        }
    }

    /// The nudge provider body. `tasks` = `list` (`None` when it failed); `ready` = `list
    /// --ready` when [`Monitor::wants_ready`]; `cli` = the `--actor agent --project` prefix.
    #[allow(
        clippy::too_many_lines,
        reason = "one decision ladder, in unipi's order (owner, question, stall, cap, autowork)"
    )]
    pub fn propose(
        &mut self,
        info: &SettleInfo,
        session: &str,
        owner: &OwnerStatus,
        tasks: Option<&[Task]>,
        ready: Option<Result<Vec<Task>, String>>,
        cli: Option<&str>,
    ) -> Outcome {
        let mut out = Outcome::default();
        if !self.armed {
            return out;
        }
        let Some(tasks) = tasks else {
            return out;
        };
        let claims: Vec<&Task> = tasks.iter().filter(|t| t.claimed_by(session)).collect();
        let ids = || {
            claims
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        // A long-horizon owner drives continuation: kanboard stays quiet. An owner that
        // STOPPED this run matters: complete → kanboard takes over again; paused/budget/other
        // → tell the user what was left.
        if owner.owner_status.as_deref() == Some("active") {
            return out;
        }
        if let Some((kind, at)) = &owner.last_stop
            && *at >= self.run_started_at
            && kind != "complete"
        {
            if !claims.is_empty() {
                let word = match kind.as_str() {
                    "paused" => "paused",
                    "budget" => "hit its budget",
                    _ => "stopped",
                };
                let ids = ids();
                self.notify_once(
                    &mut out,
                    format!("goal-stop:{ids}"),
                    format!("kanboard: goal {word} — {ids} left In Progress"),
                    "info",
                );
            }
            self.armed = false;
            return out;
        }

        if !claims.is_empty() {
            // Question heuristic: the agent ended by asking the user something.
            if !info.last_assistant_had_tool_calls
                && final_paragraph(&info.last_assistant_text).ends_with('?')
            {
                let ids = ids();
                self.notify_once(
                    &mut out,
                    format!("question:{ids}"),
                    format!("kanboard: {ids} left In Progress — the agent asked you a question"),
                    "info",
                );
                return out;
            }
            self.update_stall(info);
            if self.no_progress_runs >= STALL_LIMIT {
                let ids = ids();
                self.notify_once(
                    &mut out,
                    format!("stall:{ids}"),
                    format!("kanboard: ⚠ {ids} stalled — no progress after {STALL_LIMIT} nudges"),
                    "warning",
                );
                self.armed = false;
                return out;
            }
            let rest: Vec<&str> = claims.iter().skip(1).map(|t| t.id.as_str()).collect();
            let Some(target) = claims.iter().find(|t| {
                self.nudges_per_task.get(&t.id).copied().unwrap_or(0) < MAX_NUDGES_PER_TASK
            }) else {
                let ids = ids();
                self.notify_once(
                    &mut out,
                    format!("cap:{ids}"),
                    format!(
                        "kanboard: ⚠ {ids} stalled — nudge cap ({MAX_NUDGES_PER_TASK}) reached"
                    ),
                    "warning",
                );
                self.armed = false;
                return out;
            };
            let n = self.nudges_per_task.get(&target.id).copied().unwrap_or(0) + 1;
            let prefix = cli.unwrap_or(crate::shell::BINARY);
            let also = if rest.is_empty() {
                String::new()
            } else {
                format!(" · also open: {}", rest.join(", "))
            };
            let id = target.id.clone();
            let content = format!(
                "↻ {id} still In Progress — continue, or finish/block it ({n}/{MAX_NUDGES_PER_TASK}){also}\n`{prefix} finish {id} --comment \"<summary>\"` / `{prefix} move {id} blocked --comment \"<what you need>\"` {ANTI_POISONING_SUFFIX}"
            );
            let nudge_id = self.next_id();
            self.pending
                .insert(nudge_id.clone(), Delivered::Claim { id: id.clone(), n });
            out.proposal = Some(Proposal {
                id: nudge_id,
                priority: CLAIMS_PRIORITY,
                custom_type: CLAIMS_NUDGE_CUSTOM_TYPE,
                content,
                title: "Unfinished task",
                synopsis: id,
                warning: true,
            });
            return out;
        }

        if self.autowork {
            self.update_stall(info);
            if self.no_progress_runs >= STALL_LIMIT {
                self.notify_once(
                    &mut out,
                    "autowork-stall".into(),
                    format!(
                        "kanboard: ⚠ autowork stalled — no progress after {STALL_LIMIT} offers"
                    ),
                    "warning",
                );
                self.autowork = false;
                out.autowork_off = true;
                self.armed = false;
                return out;
            }
            let Some(Ok(ready)) = ready else {
                return out;
            };
            let ready: Vec<Task> = ready.into_iter().filter(|t| t.status == "todo").collect();
            if let Some(next) = ready.first() {
                // A task that left the ready list gets a fresh offer budget.
                self.offers_per_task
                    .retain(|id, _| ready.iter().any(|t| &t.id == id));
                let offers = self.offers_per_task.get(&next.id).copied().unwrap_or(0) + 1;
                if offers > MAX_OFFERS_PER_TASK {
                    self.notify_once(
                        &mut out,
                        format!("autowork-cap:{}", next.id),
                        format!("kanboard: ⚠ autowork stalled on {}", next.id),
                        "warning",
                    );
                    self.autowork = false;
                    out.autowork_off = true;
                    self.armed = false;
                    return out;
                }
                let nudge_id = self.next_id();
                self.pending.insert(
                    nudge_id.clone(),
                    Delivered::Offer {
                        id: next.id.clone(),
                        offers,
                    },
                );
                out.proposal = Some(Proposal {
                    id: nudge_id,
                    priority: AUTOWORK_PRIORITY,
                    custom_type: AUTOWORK_NUDGE_CUSTOM_TYPE,
                    content: format!(
                        "↻ next ready: {} {} — show it, start it, work it (autowork) {ANTI_POISONING_SUFFIX}",
                        next.id,
                        next.shown_title()
                    ),
                    title: "Next task",
                    synopsis: next.id.clone(),
                    warning: false,
                });
                return out;
            }
            let in_review = tasks.iter().filter(|t| t.status == "in_review").count();
            let blocked = tasks.iter().filter(|t| t.status == "blocked").count();
            self.notify_once(
                &mut out,
                "autowork-done".into(),
                format!("kanboard: autowork done · {in_review} in review · {blocked} blocked"),
                "info",
            );
            self.autowork = false;
            out.autowork_off = true;
            self.armed = false;
        }
        out
    }

    /// The arbiter delivered nudge `id`: counts on DELIVERY, not proposal.
    pub fn delivered(&mut self, id: &str) {
        match self.pending.remove(id) {
            Some(Delivered::Claim { id, n }) => {
                self.nudges_per_task.insert(id, n);
                self.nudge_delivered = true;
            }
            Some(Delivered::Offer { id, offers }) => {
                self.offers_per_task.insert(id, offers);
                self.nudge_delivered = true;
            }
            None => {}
        }
        self.pending.clear();
    }

    /// Delivered continuation nudges per task (tests).
    #[must_use]
    pub fn nudges(&self, id: &str) -> u32 {
        self.nudges_per_task.get(id).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Run;

    const S: &str = "oino-s1";

    fn claim(id: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("title {id}"),
            status: "in_progress".into(),
            run: Some(Run {
                session: Some(S.into()),
                ..Run::default()
            }),
            ..Task::default()
        }
    }

    fn task(id: &str, status: &str) -> Task {
        Task {
            id: id.into(),
            title: format!("title {id}"),
            status: status.into(),
            ..Task::default()
        }
    }

    fn busy() -> SettleInfo {
        SettleInfo {
            last_assistant_text: "working".into(),
            last_assistant_had_tool_calls: false,
            tool_calls_this_run: 3,
        }
    }

    fn propose(m: &mut Monitor, tasks: &[Task], info: &SettleInfo) -> Outcome {
        m.propose(
            info,
            S,
            &OwnerStatus::default(),
            Some(tasks),
            None,
            Some("/b --actor agent --project p"),
        )
    }

    #[test]
    fn not_armed_is_none_and_arm_enables_the_claims_nudge() {
        let mut m = Monitor::default();
        assert!(propose(&mut m, &[claim("A-1")], &busy()).proposal.is_none());
        m.arm();
        let p = propose(&mut m, &[claim("A-1")], &busy()).proposal.unwrap();
        assert_eq!(p.priority, CLAIMS_PRIORITY);
        assert_eq!(p.custom_type, CLAIMS_NUDGE_CUSTOM_TYPE);
        assert!(p.content.starts_with("↻ A-1 still In Progress — continue, or finish/block it (1/5)\n`/b --actor agent --project p finish A-1"));
        let j = p.to_json();
        assert_eq!(j["details"]["oinoProvenance"]["severity"], "warning");
        assert_eq!(j["display"], true);
    }

    #[test]
    fn a_second_claim_rides_along() {
        let mut m = Monitor::default();
        m.arm();
        let p = propose(&mut m, &[claim("A-1"), claim("A-2")], &busy())
            .proposal
            .unwrap();
        assert!(p.content.contains("(1/5) · also open: A-2\n"));
    }

    #[test]
    fn count_grows_only_on_delivery_capped_at_five() {
        let mut m = Monitor::default();
        m.arm();
        let tasks = [claim("A-1")];
        for _ in 0..3 {
            propose(&mut m, &tasks, &busy());
        }
        assert_eq!(m.nudges("A-1"), 0);
        for n in 1..=5 {
            m.on_agent_start(0);
            let p = propose(&mut m, &tasks, &busy()).proposal.unwrap();
            assert!(p.content.contains(&format!("({n}/5)")));
            m.delivered(&p.id);
        }
        let out = propose(&mut m, &tasks, &busy());
        assert!(out.proposal.is_none());
        assert_eq!(
            out.notices[0].text,
            "kanboard: ⚠ A-1 stalled — nudge cap (5) reached"
        );
        assert!(!m.armed());
    }

    #[test]
    fn an_active_owner_silences_the_monitor() {
        let mut m = Monitor::default();
        m.arm();
        let owner = OwnerStatus {
            owner_status: Some("active".into()),
            last_stop: None,
        };
        assert!(
            m.propose(&busy(), S, &owner, Some(&[claim("A-1")]), None, None)
                .proposal
                .is_none()
        );
        assert!(m.armed());
    }

    #[test]
    fn an_owner_stop_this_run_notices_and_disarms_complete_keeps_nudging() {
        let mut m = Monitor::default();
        m.arm();
        m.on_agent_start(100);
        let paused = OwnerStatus {
            owner_status: None,
            last_stop: Some(("paused".into(), 150)),
        };
        let out = m.propose(&busy(), S, &paused, Some(&[claim("A-1")]), None, None);
        assert_eq!(
            out.notices[0].text,
            "kanboard: goal paused — A-1 left In Progress"
        );
        assert!(!m.armed());
        m.arm();
        let budget = OwnerStatus {
            owner_status: None,
            last_stop: Some(("budget".into(), 150)),
        };
        assert!(
            m.propose(&busy(), S, &budget, Some(&[claim("A-1")]), None, None)
                .notices[0]
                .text
                .contains("hit its budget")
        );
        m.arm();
        let complete = OwnerStatus {
            owner_status: None,
            last_stop: Some(("complete".into(), 150)),
        };
        assert!(
            m.propose(&busy(), S, &complete, Some(&[claim("A-1")]), None, None)
                .proposal
                .is_some()
        );
        // A stop older than the run is ignored.
        let old = OwnerStatus {
            owner_status: None,
            last_stop: Some(("paused".into(), 50)),
        };
        assert!(
            m.propose(&busy(), S, &old, Some(&[claim("A-1")]), None, None)
                .proposal
                .is_some()
        );
    }

    #[test]
    fn question_heuristic_notices_without_counting() {
        let mut m = Monitor::default();
        m.arm();
        let q = SettleInfo {
            last_assistant_text: "Did it.\n\nShould I also update the docs?".into(),
            last_assistant_had_tool_calls: false,
            tool_calls_this_run: 2,
        };
        let out = propose(&mut m, &[claim("A-1")], &q);
        assert!(out.proposal.is_none());
        assert_eq!(
            out.notices[0].text,
            "kanboard: A-1 left In Progress — the agent asked you a question"
        );
        assert!(
            propose(&mut m, &[claim("A-1")], &q).notices.is_empty(),
            "once"
        );
        assert!(m.armed());
    }

    #[test]
    fn stall_after_two_idle_nudged_runs_and_progress_resets() {
        let mut m = Monitor::default();
        m.arm();
        let idle = SettleInfo::default();
        let tasks = [claim("A-1")];
        let p = propose(&mut m, &tasks, &busy()).proposal.unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        let p = propose(&mut m, &tasks, &idle).proposal.unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        // A productive run resets.
        let p = propose(&mut m, &tasks, &busy()).proposal.unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        let p = propose(&mut m, &tasks, &idle).proposal.unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        let out = propose(&mut m, &tasks, &idle);
        assert!(out.proposal.is_none());
        assert_eq!(out.notices[0].level, "warning");
        assert!(
            out.notices[0]
                .text
                .contains("stalled — no progress after 2 nudges")
        );
    }

    #[test]
    fn aborted_or_errored_runs_disarm() {
        let mut m = Monitor::default();
        m.arm();
        m.on_agent_end(&[json!({"role": "assistant", "stopReason": "stop"})]);
        assert!(m.armed());
        m.on_agent_end(&[
            json!({"role": "assistant", "stopReason": "aborted"}),
            json!({"role": "toolResult"}),
        ]);
        assert!(!m.armed());
        m.arm();
        m.on_agent_end(&[json!({"role": "assistant", "stopReason": "error"})]);
        assert!(!m.armed());
    }

    #[test]
    fn a_prompt_naming_our_claim_rearms() {
        let mut m = Monitor::default();
        let tasks = [claim("A-1"), task("A-2", "todo")];
        let ids = Monitor::prompt_ids("look at A-2").unwrap();
        m.on_user_prompt(&ids, &tasks, S);
        assert!(!m.armed());
        let ids = Monitor::prompt_ids("continue A-1 please").unwrap();
        m.on_user_prompt(&ids, &tasks, S);
        assert!(m.armed());
        assert!(Monitor::prompt_ids(&format!("A-1 {ANTI_POISONING_SUFFIX}")).is_none());
        assert!(Monitor::prompt_ids("no ids").is_none());
    }

    #[test]
    fn autowork_claims_first_then_ready_then_done() {
        let mut m = Monitor::default();
        m.set_autowork(true);
        m.arm();
        let p = propose(&mut m, &[claim("A-1"), task("A-2", "todo")], &busy())
            .proposal
            .unwrap();
        assert_eq!(p.priority, CLAIMS_PRIORITY);
        let board = [
            task("A-2", "todo"),
            task("A-3", "in_review"),
            task("A-4", "blocked"),
        ];
        let out = m.propose(
            &busy(),
            S,
            &OwnerStatus::default(),
            Some(&board),
            Some(Ok(vec![task("A-2", "todo")])),
            None,
        );
        let p = out.proposal.unwrap();
        assert_eq!(p.priority, AUTOWORK_PRIORITY);
        assert!(
            p.content.starts_with(
                "↻ next ready: A-2 title A-2 — show it, start it, work it (autowork) "
            )
        );
        let out = m.propose(
            &busy(),
            S,
            &OwnerStatus::default(),
            Some(&board),
            Some(Ok(vec![])),
            None,
        );
        assert!(out.autowork_off);
        assert_eq!(
            out.notices[0].text,
            "kanboard: autowork done · 1 in review · 1 blocked"
        );
        assert!(!m.autowork());
        // Disarmed: nothing more, the done notice fires once.
        let out = m.propose(
            &busy(),
            S,
            &OwnerStatus::default(),
            Some(&board),
            Some(Ok(vec![])),
            None,
        );
        assert_eq!(out.notices, [] as [Notice; 0]);
    }

    #[test]
    fn autowork_stall_and_offer_cap() {
        let mut m = Monitor::default();
        m.set_autowork(true);
        m.arm();
        let ready = || Some(Ok(vec![task("A-2", "todo")]));
        let board = [task("A-2", "todo")];
        for _ in 0..3 {
            let p = m
                .propose(
                    &busy(),
                    S,
                    &OwnerStatus::default(),
                    Some(&board),
                    ready(),
                    None,
                )
                .proposal
                .unwrap();
            m.delivered(&p.id);
            m.on_agent_start(0);
        }
        let out = m.propose(
            &busy(),
            S,
            &OwnerStatus::default(),
            Some(&board),
            ready(),
            None,
        );
        assert_eq!(out.notices[0].text, "kanboard: ⚠ autowork stalled on A-2");
        assert!(out.autowork_off);

        let mut m = Monitor::default();
        m.set_autowork(true);
        m.arm();
        let idle = SettleInfo::default();
        let p = m
            .propose(
                &busy(),
                S,
                &OwnerStatus::default(),
                Some(&board),
                ready(),
                None,
            )
            .proposal
            .unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        let p = m
            .propose(
                &idle,
                S,
                &OwnerStatus::default(),
                Some(&board),
                ready(),
                None,
            )
            .proposal
            .unwrap();
        m.delivered(&p.id);
        m.on_agent_start(0);
        let out = m.propose(
            &idle,
            S,
            &OwnerStatus::default(),
            Some(&board),
            ready(),
            None,
        );
        assert!(
            out.notices[0]
                .text
                .contains("autowork stalled — no progress after 2 offers")
        );
        assert!(out.autowork_off);
    }

    #[test]
    fn a_task_that_left_ready_gets_a_fresh_offer_budget() {
        let mut m = Monitor::default();
        m.set_autowork(true);
        m.arm();
        let board = [task("A-2", "todo")];
        for _ in 0..3 {
            let p = m
                .propose(
                    &busy(),
                    S,
                    &OwnerStatus::default(),
                    Some(&board),
                    Some(Ok(vec![task("A-2", "todo")])),
                    None,
                )
                .proposal
                .unwrap();
            m.delivered(&p.id);
        }
        let p = m
            .propose(
                &busy(),
                S,
                &OwnerStatus::default(),
                Some(&board),
                Some(Ok(vec![task("A-3", "todo")])),
                None,
            )
            .proposal
            .unwrap();
        m.delivered(&p.id);
        let p = m.propose(
            &busy(),
            S,
            &OwnerStatus::default(),
            Some(&board),
            Some(Ok(vec![task("A-2", "todo")])),
            None,
        );
        assert!(p.proposal.is_some(), "A-2 left the list, so it starts over");
    }

    #[test]
    fn board_errors_are_none() {
        let mut m = Monitor::default();
        m.arm();
        let out = m.propose(&busy(), S, &OwnerStatus::default(), None, None, None);
        assert_eq!(out, Outcome::default());
    }

    #[test]
    fn last_assistant_extracts_text_and_tools() {
        let msgs = [
            json!({"role": "assistant", "content": [{"type": "text", "text": "a"}, {"type": "toolCall", "id": "1"}]}),
        ];
        assert_eq!(last_assistant(&msgs), ("a".into(), true));
        assert_eq!(last_assistant(&[]), (String::new(), false));
    }
}
