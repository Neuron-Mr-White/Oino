//! Goal state machine, verifier and goal prompts (unipi `engine/goal-state.ts`,
//! `engine/verifier.ts`, `prompts/goal.ts`, `engine/task-gate.ts`, `tools/goal.ts` texts).

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default turn cap.
pub const DEFAULT_MAX_TURNS: u32 = 50;
/// Default stall cap.
pub const DEFAULT_STALL_CAP: u32 = 8;
/// Turn ceiling.
pub const MAX_TURNS_CEILING: u32 = 200;
/// Stall ceiling.
pub const STALL_CAP_CEILING: u32 = 50;
/// Minimum token budget.
pub const TOKEN_BUDGET_MINIMUM: u64 = 1_000;
/// Blocked threshold.
pub const BLOCKED_PROPOSAL_THRESHOLD: u32 = 3;
/// `not_met` cap.
pub const NOT_MET_STREAK_CAP: u32 = 5;
/// `inconclusive` cap.
pub const INCONCLUSIVE_STREAK_CAP: u32 = 2;
/// Status audit cadence.
pub const AUDIT_EVERY_N_TURNS: u32 = 5;
/// Recovery re-deliveries.
pub const MAX_RECOVERY_REDELIVERIES: u32 = 2;
/// Wait backoff base.
pub const WAIT_BACKOFF_BASE_MS: u64 = 5_000;
/// Wait backoff cap.
pub const WAIT_BACKOFF_MAX_MS: u64 = 300_000;
/// Task reminders per goal.
pub const TASK_REMINDERS_PER_GOAL: u32 = 2;
const PENDING_TEXT_LIMIT: usize = 16_000;

const KICKOFF: &str = include_str!("../assets/kickoff.md");
const VERIFIER_SYSTEM: &str = include_str!("../assets/verifier.md");
/// unipi `CONTINUATION_HINT`.
pub const CONTINUATION_HINT: &str = "Continue working toward the active thread goal from the current conversation state. Reinspect the current evidence, make concrete progress, and follow the goal contract from the kickoff context.";
/// unipi `NO_PROGRESS_NUDGE`.
pub const NO_PROGRESS_NUDGE: &str = "No-progress guard: your latest response repeated an earlier stopping point. Choose a materially different next action that advances the objective and execute it before reporting back.";
/// unipi `NO_TOOL_NUDGE`.
pub const NO_TOOL_NUDGE: &str = "No-progress guard: this turn ended without using a single tool. Restating a plan or intention is not progress. Inspect the current evidence with tools and execute the next concrete action before reporting back.";
/// unipi `TERMINAL_AUDIT`.
pub const TERMINAL_AUDIT: &str = "Goal status audit (scheduled checkpoint): call get_goal and compare the full objective with current authoritative evidence. Propose completion only if proven; otherwise keep making concrete progress.";
/// unipi `WRAP_UP_PROMPT`.
pub const WRAP_UP_PROMPT: &str = "The active goal reached a budget limit and the loop has stopped. Summarize for the user: what was completed, what remains, and the exact next step to resume (the goal state is durable — /goal resume).";
/// unipi `RECOVERY_FRAGMENT`.
pub const RECOVERY_FRAGMENT: &str = "Goal recovery: this goal is resuming after an interrupted or retracted turn; the conversation may be incomplete. Call get_goal first and treat its state as the durable source of truth before continuing.";
/// unipi `TASK_GATE_REMINDER`.
pub const TASK_GATE_REMINDER: &str = "[Task reminder] Actionable session tasks remain. Reconcile them before stopping: finish them with real evidence, or update their status truthfully. A task is advisory and never overrides files, tests, artifacts, or verifier evidence.";

/// Lowercase hex SHA-256 of `data` (unipi `digestObjective`).
#[must_use]
pub fn digest_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(data)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// unipi `GoalStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Running.
    Active,
    /// Waiting on an external event.
    Waiting,
    /// Parked.
    Paused,
    /// Done.
    Complete,
    /// Blocked.
    Blocked,
    /// Budget reached.
    BudgetLimited,
    /// Provider quota.
    UsageLimited,
}

impl Status {
    /// Terminal.
    #[must_use]
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Blocked | Self::BudgetLimited | Self::UsageLimited
        )
    }
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Complete => "complete",
            Self::Blocked => "blocked",
            Self::BudgetLimited => "budget_limited",
            Self::UsageLimited => "usage_limited",
        }
    }
}

/// unipi `GoalPendingContinuation`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// Text.
    pub text: String,
    /// Revision.
    pub revision: u64,
    /// Attempts.
    pub attempts: u32,
}

/// unipi `GoalState`.
#[allow(clippy::struct_excessive_bools, reason = "mirrors unipi GoalState")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    /// Id.
    pub goal_id: String,
    /// Objective.
    pub objective: String,
    /// SHA-256 of the objective.
    pub objective_digest: String,
    /// Status.
    pub status: Status,
    /// Reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Token budget.
    pub token_budget: Option<u64>,
    /// Baseline not yet sampled.
    pub tokens_baseline_pending: bool,
    /// Estimated counts.
    #[serde(default)]
    pub tokens_estimated: bool,
    /// Tokens at start.
    pub tokens_at_start: u64,
    /// Tokens now.
    pub tokens_now: u64,
    /// Turn.
    pub turn: u32,
    /// Max turns.
    pub max_turns: u32,
    /// No-progress streak.
    pub no_progress_streak: u32,
    /// Stall cap.
    pub stall_cap: u32,
    /// `not_met` streak.
    pub not_met_streak: u32,
    /// Inconclusive streak.
    #[serde(default)]
    pub inconclusive_streak: u32,
    /// Blocked proposals in a row.
    pub blocked_proposal_streak: u32,
    /// Lease generation.
    pub generation: u64,
    /// Revision.
    pub revision: u64,
    /// Updated (ISO).
    pub updated_at: String,
    /// Kickoff delivered.
    #[serde(default)]
    pub kickoff_delivered: bool,
    /// Wrap-up delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_up_delivered_revision: Option<u64>,
    /// Durable continuation outbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_continuation: Option<Pending>,
}

/// unipi `GoalSettlement`.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi GoalSettlement"
)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settlement {
    /// A completion claim with this verdict (`met|not_met|impossible|inconclusive`).
    pub verdict: Option<String>,
    /// Evaluator unavailable.
    pub verifier_unavailable: bool,
    /// Progress (`None` = unknown).
    pub made_progress: Option<bool>,
    /// Waiting.
    pub waiting: bool,
    /// Tokens.
    pub tokens_now: Option<u64>,
    /// Blocked proposal.
    pub blocked_proposal: bool,
    /// Safety refusal.
    pub safety_refusal: bool,
}

/// The goal machine (one goal per session, persisted as `goal.json` `{version: 1, goal}`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// Current goal.
    pub goal: Option<Goal>,
}

/// unipi `isConditionWithinLimit`.
#[must_use]
pub fn within_limit(v: &str) -> bool {
    v.encode_utf16().count() <= 500 && v.len() <= 1_500
}

impl Machine {
    fn bump(goal: &Goal, now: &str, f: impl FnOnce(&mut Goal)) -> Goal {
        let mut g = goal.clone();
        g.pending_continuation = None;
        f(&mut g);
        g.revision = goal.revision + 1;
        now.clone_into(&mut g.updated_at);
        g
    }

    /// The non-terminal goal.
    #[must_use]
    pub fn active(&self) -> Option<&Goal> {
        self.goal.as_ref().filter(|g| !g.status.terminal())
    }

    /// unipi `create`: `Err(goal)` when an unfinished goal exists.
    ///
    /// # Errors
    ///
    /// The unfinished goal, or a text-limit error.
    pub fn create(
        &mut self,
        id: String,
        objective: &str,
        budget: Option<u64>,
        max_turns: Option<u32>,
        now: &str,
    ) -> Result<Goal, std::result::Result<Box<Goal>, String>> {
        let t = objective.trim();
        if t.is_empty() || !within_limit(t) {
            return Err(Err("Goal objective exceeds its shared text limit".into()));
        }
        if let Some(g) = self.active() {
            return Err(Ok(Box::new(g.clone())));
        }
        let g = Goal {
            goal_id: id,
            objective: t.to_owned(),
            objective_digest: digest_hex(t.as_bytes()),
            status: Status::Active,
            reason: None,
            token_budget: budget,
            tokens_baseline_pending: true,
            tokens_estimated: false,
            tokens_at_start: 0,
            tokens_now: 0,
            turn: 0,
            max_turns: max_turns
                .unwrap_or(DEFAULT_MAX_TURNS)
                .min(MAX_TURNS_CEILING),
            no_progress_streak: 0,
            stall_cap: DEFAULT_STALL_CAP,
            not_met_streak: 0,
            inconclusive_streak: 0,
            blocked_proposal_streak: 0,
            generation: 0,
            revision: 0,
            updated_at: now.to_owned(),
            kickoff_delivered: false,
            wrap_up_delivered_revision: None,
            pending_continuation: None,
        };
        self.goal = Some(g.clone());
        Ok(g)
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "every transition returns the new state as Option"
    )]
    fn set(&mut self, g: Goal) -> Option<Goal> {
        self.goal = Some(g.clone());
        Some(g)
    }

    /// Pause (active/waiting only).
    pub fn pause(&mut self, reason: &str, now: &str) -> Option<Goal> {
        let g = self
            .goal
            .as_ref()
            .filter(|g| matches!(g.status, Status::Active | Status::Waiting))?;
        let n = Self::bump(g, now, |n| {
            n.status = Status::Paused;
            n.reason = Some(reason.to_owned());
            n.generation += 1;
        });
        self.set(n)
    }
    /// Resume a paused goal.
    pub fn resume(&mut self, now: &str) -> Option<Goal> {
        let g = self.goal.as_ref().filter(|g| g.status == Status::Paused)?;
        let n = Self::bump(g, now, |n| {
            n.status = Status::Active;
            n.reason = None;
            n.generation += 1;
            n.blocked_proposal_streak = 0;
        });
        self.set(n)
    }
    /// Wake a waiting goal.
    pub fn wake(&mut self, now: &str) -> Option<Goal> {
        let g = self.goal.as_ref().filter(|g| g.status == Status::Waiting)?;
        let n = Self::bump(g, now, |n| n.status = Status::Active);
        self.set(n)
    }
    /// User clear (`complete(user_requested)`).
    pub fn clear(&mut self, now: &str) -> Option<Goal> {
        let g = self.goal.as_ref().filter(|g| !g.status.terminal())?;
        let n = Self::bump(g, now, |n| {
            n.status = Status::Complete;
            n.reason = Some("complete(user_requested)".into());
            n.generation += 1;
        });
        self.set(n)
    }
    /// Budget edit.
    pub fn set_budget(&mut self, budget: Option<u64>, now: &str) -> Option<Goal> {
        let g = self.goal.as_ref()?;
        let n = Self::bump(g, now, |n| n.token_budget = budget);
        self.set(n)
    }
    /// Mark the kickoff delivered.
    pub fn mark_kickoff(&mut self, now: &str) -> Option<Goal> {
        let g = self.goal.as_ref().filter(|g| !g.kickoff_delivered)?;
        let n = Self::bump(g, now, |n| n.kickoff_delivered = true);
        self.set(n)
    }
    /// Durable outbox write (no revision bump).
    pub fn record_pending(&mut self, text: &str, attempts: u32) {
        if let Some(g) = self
            .goal
            .as_mut()
            .filter(|g| matches!(g.status, Status::Active | Status::Waiting))
        {
            g.pending_continuation = Some(Pending {
                text: text.chars().take(PENDING_TEXT_LIMIT).collect(),
                revision: g.revision,
                attempts,
            });
        }
    }
    /// Clears the outbox.
    pub fn clear_pending(&mut self) {
        if let Some(g) = self.goal.as_mut() {
            g.pending_continuation = None;
        }
    }

    /// unipi `settleTurn`.
    #[allow(clippy::too_many_lines, reason = "one transition table, as unipi")]
    pub fn settle(&mut self, s: &Settlement, now: &str) -> Option<Goal> {
        let g = self
            .goal
            .clone()
            .filter(|g| matches!(g.status, Status::Active | Status::Waiting))?;
        let stop = |status: Status, reason: &str, f: &dyn Fn(&mut Goal)| {
            Self::bump(&g, now, |n| {
                n.status = status;
                n.reason = Some(reason.to_owned());
                f(n);
            })
        };
        let mut inconclusive = g.inconclusive_streak;
        let mut not_met = g.not_met_streak;
        if let Some(verdict) = &s.verdict {
            if s.verifier_unavailable {
                return self.set(stop(Status::Paused, "paused(verifier_unavailable)", &|n| {
                    n.inconclusive_streak = 0;
                }));
            }
            match verdict.as_str() {
                "met" => {
                    return self.set(stop(Status::Complete, "complete(verifier_met)", &|n| {
                        n.inconclusive_streak = 0;
                    }));
                }
                "impossible" => {
                    return self.set(stop(
                        Status::Blocked,
                        "blocked(verifier_impossible)",
                        &|n| n.inconclusive_streak = 0,
                    ));
                }
                "not_met" => {
                    not_met += 1;
                    inconclusive = 0;
                    if not_met >= NOT_MET_STREAK_CAP {
                        return self.set(stop(
                            Status::Paused,
                            "paused(no_progress_after_completion_claim)",
                            &|n| {
                                n.not_met_streak = not_met;
                                n.inconclusive_streak = 0;
                            },
                        ));
                    }
                }
                _ => {
                    inconclusive += 1;
                    if inconclusive >= INCONCLUSIVE_STREAK_CAP {
                        return self.set(stop(
                            Status::Paused,
                            "paused(verifier_inconclusive)",
                            &|n| n.inconclusive_streak = inconclusive,
                        ));
                    }
                }
            }
        } else {
            inconclusive = 0;
        }
        if s.safety_refusal {
            return self.set(stop(Status::Blocked, "blocked(safety_policy)", &|_| {}));
        }
        let mut blocked = g.blocked_proposal_streak;
        if s.blocked_proposal {
            blocked += 1;
            if blocked >= BLOCKED_PROPOSAL_THRESHOLD {
                return self.set(stop(Status::Blocked, "blocked(threshold_3turns)", &|n| {
                    n.blocked_proposal_streak = blocked;
                }));
            }
        } else {
            blocked = 0;
        }
        let (mut at_start, mut tokens_now, mut pending) =
            (g.tokens_at_start, g.tokens_now, g.tokens_baseline_pending);
        if let Some(t) = s.tokens_now {
            if pending {
                at_start = t;
                tokens_now = t;
                pending = false;
            } else {
                tokens_now = tokens_now.max(t);
            }
            if g.token_budget.is_some_and(|b| tokens_now - at_start >= b) {
                return self.set(stop(Status::BudgetLimited, "budget_limited(token)", &|n| {
                    n.tokens_at_start = at_start;
                    n.tokens_now = tokens_now;
                    n.tokens_baseline_pending = false;
                }));
            }
        }
        let turn = g.turn + 1;
        if turn >= g.max_turns {
            return self.set(stop(
                Status::BudgetLimited,
                "budget_limited(max_turns)",
                &|n| n.turn = turn,
            ));
        }
        let mut no_progress = g.no_progress_streak;
        match s.made_progress {
            Some(true) => no_progress = 0,
            Some(false) => {
                no_progress += 1;
                if no_progress >= g.stall_cap {
                    return self.set(stop(Status::Paused, "paused(no_progress)", &|n| {
                        n.turn = turn;
                        n.no_progress_streak = no_progress;
                        n.not_met_streak = not_met;
                        n.blocked_proposal_streak = blocked;
                    }));
                }
            }
            None => {}
        }
        let (status, reason) = if s.waiting {
            (Status::Waiting, Some("waiting(external_event)".to_owned()))
        } else if g.status == Status::Waiting {
            (Status::Active, None)
        } else {
            (g.status, g.reason.clone())
        };
        let n = Self::bump(&g, now, |n| {
            n.status = status;
            n.reason = reason;
            n.tokens_at_start = at_start;
            n.tokens_now = tokens_now;
            n.tokens_baseline_pending = pending;
            n.turn = turn;
            n.no_progress_streak = no_progress;
            n.not_met_streak = not_met;
            n.inconclusive_streak = inconclusive;
            n.blocked_proposal_streak = blocked;
        });
        self.set(n)
    }
}

/// unipi `escapeXmlText`.
#[must_use]
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// unipi `renderKickoff`.
#[must_use]
pub fn kickoff(objective: &str) -> String {
    KICKOFF.replace("{{objective}}", &escape_xml(objective))
}

/// unipi `renderContinuationHint`.
#[must_use]
pub fn continuation_hint(g: &Goal, verifier_reason: Option<&str>) -> String {
    let np = if g.no_progress_streak > 0 {
        format!(", {}/{} no-progress", g.no_progress_streak, g.stall_cap)
    } else {
        String::new()
    };
    let budget = match g.token_budget {
        Some(b) if !g.tokens_baseline_pending => format!(
            ", {}/{b} tok{}",
            g.tokens_now.saturating_sub(g.tokens_at_start),
            if g.tokens_estimated { " (est.)" } else { "" }
        ),
        _ => String::new(),
    };
    format!(
        "{CONTINUATION_HINT}{}\nGoal: \"{}\" (turn {}/{}{np}{budget})",
        verifier_reason
            .map(|r| format!("\nEvaluation: {r}"))
            .unwrap_or_default(),
        g.objective,
        g.turn,
        g.max_turns
    )
}

/// Which continuation message to send (unipi `selectHint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// Plain hint.
    Hint,
    /// Repeated stopping point.
    NoProgress,
    /// No tool used.
    NoTool,
    /// Scheduled audit.
    Audit,
    /// Recovery.
    Recovery,
}

/// unipi `selectHint`.
#[must_use]
pub fn select_hint(g: &Goal, tool_calls: u32, recovery: bool) -> Hint {
    if recovery {
        Hint::Recovery
    } else if tool_calls == 0 {
        Hint::NoTool
    } else if g.no_progress_streak >= 2 {
        Hint::NoProgress
    } else if g.turn > 0 && g.turn.is_multiple_of(AUDIT_EVERY_N_TURNS) {
        Hint::Audit
    } else {
        Hint::Hint
    }
}

/// unipi `waitBackoffMs`.
#[must_use]
pub fn wait_backoff_ms(waits: u32) -> u64 {
    let exp = waits.saturating_sub(1).min(20);
    (WAIT_BACKOFF_BASE_MS.saturating_mul(1u64 << exp)).min(WAIT_BACKOFF_MAX_MS)
}

/// unipi `renderTaskReminder` + `attachTaskReminder`.
#[must_use]
pub fn task_reminder(tasks: &[String]) -> String {
    let lines: Vec<String> = tasks
        .iter()
        .take(10)
        .map(|t| {
            let flat = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if flat.chars().count() > 120 {
                format!("- {}…", flat.chars().take(119).collect::<String>())
            } else {
                format!("- {flat}")
            }
        })
        .collect();
    format!("{TASK_GATE_REMINDER}\n{}", lines.join("\n"))
}

/// Evidence for the verifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    /// Claim.
    pub claim: Option<String>,
    /// Changed files.
    pub files: Vec<String>,
    /// Commands.
    pub commands: Vec<String>,
    /// `(role, text)` tail.
    pub tail: Vec<(String, String)>,
}

fn bounded(items: &[String], cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut total = 0;
    for item in items.iter().take(100) {
        let slice: String = item.chars().take(cap.saturating_sub(total)).collect();
        if slice.is_empty() {
            break;
        }
        total += slice.chars().count();
        out.push(slice);
    }
    out
}

/// unipi `assembleEvidenceBrief`.
#[must_use]
pub fn evidence_brief(objective: &str, digest: &str, e: &Evidence) -> String {
    let files = bounded(&e.files, 2000);
    let commands = bounded(&e.commands, 2000);
    let tail: Vec<String> = e.tail[e.tail.len().saturating_sub(5)..]
        .iter()
        .map(|(r, t)| format!("{r}: {}", t.chars().take(800).collect::<String>()))
        .collect();
    let mut lines = vec![
        "--- OBJECTIVE ---".to_owned(),
        objective.to_owned(),
        "--- OBJECTIVE DIGEST ---".to_owned(),
        digest.to_owned(),
    ];
    if let Some(c) = &e.claim {
        lines.push("--- WORKER CLAIM (untrusted) ---".into());
        lines.push(c.chars().take(800).collect());
    }
    lines.push("--- CHANGED FILES ---".into());
    lines.push(if files.is_empty() {
        "(none reported)".into()
    } else {
        files.join("\n")
    });
    lines.push("--- COMMANDS RUN ---".into());
    lines.push(if commands.is_empty() {
        "(none reported)".into()
    } else {
        commands.join("\n")
    });
    lines.push("--- RECENT CONVERSATION TAIL ---".into());
    lines.push(if tail.is_empty() {
        "(empty)".into()
    } else {
        tail.join("\n")
    });
    lines.push("--- YOUR VERDICT (JSON only) ---".into());
    lines.join("\n")
}

/// unipi `buildVerificationPrompt`.
#[must_use]
pub fn verification_prompt(brief: &str) -> String {
    format!("{VERIFIER_SYSTEM}\n\n{brief}\n")
}

/// unipi `VerificationVerdict`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// `met | not_met | impossible | inconclusive`.
    pub verdict: String,
    /// Reason.
    pub reason: String,
    /// Missing evidence.
    pub missing: Vec<String>,
    /// Evaluator failed.
    pub failed: bool,
}

impl Verdict {
    /// An evaluator failure.
    #[must_use]
    pub fn failure(reason: &str) -> Self {
        Self {
            verdict: "inconclusive".into(),
            reason: reason.into(),
            missing: Vec::new(),
            failed: true,
        }
    }
}

/// unipi `parseVerification`.
#[must_use]
pub fn parse_verification(raw: &str) -> Verdict {
    let fallback = Verdict::failure("Evaluator produced unparseable output");
    let Some(start) = raw.find('{') else {
        return fallback;
    };
    let mut candidates = Vec::new();
    for (i, _) in raw.match_indices('{') {
        if let Some(end) = raw[i..].find('}') {
            candidates.push(&raw[i..=i + end]);
        }
    }
    let _ = start;
    let pick = candidates
        .iter()
        .find(|c| c.contains("\"verdict\""))
        .or_else(|| candidates.first());
    let Some(text) = pick else { return fallback };
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Verdict::failure("Evaluator JSON parse failed");
    };
    let verdict = v.get("verdict").and_then(Value::as_str).unwrap_or_default();
    if !matches!(verdict, "met" | "not_met" | "impossible" | "inconclusive") {
        return fallback;
    }
    Verdict {
        verdict: verdict.to_owned(),
        reason: v
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| !r.trim().is_empty())
            .map_or_else(
                || "No reason provided".to_owned(),
                |r| r.chars().take(200).collect(),
            ),
        missing: v
            .get("missing")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(10)
            .map(|m| m.chars().take(200).collect())
            .collect(),
        failed: false,
    }
}

/// `get_goal` JSON (unipi `GoalToolset.get`).
#[must_use]
pub fn get_goal_json(g: &Goal, pending: Option<&str>) -> String {
    let mut v = json!({
        "goal_id": g.goal_id, "objective": g.objective, "status": g.status.as_str(),
    });
    if let Some(r) = &g.reason {
        v["reason"] = json!(r);
    }
    for (k, val) in [
        ("turn", json!(g.turn)),
        ("max_turns", json!(g.max_turns)),
        ("no_progress_streak", json!(g.no_progress_streak)),
        ("stall_cap", json!(g.stall_cap)),
        ("token_budget", json!(g.token_budget)),
        (
            "tokens_used",
            if g.tokens_baseline_pending {
                Value::Null
            } else {
                json!(g.tokens_now - g.tokens_at_start)
            },
        ),
        ("tokens_estimated", json!(g.tokens_estimated)),
        ("expected_goal_id", json!(g.goal_id)),
        ("expected_updated_at", json!(g.updated_at)),
        ("updated_at", json!(g.updated_at)),
    ] {
        v[k] = val;
    }
    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
    if let Some(kind) = pending {
        let _ = write!(s, " Pending proposal: {kind} (verification pending).");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        let mut m = Machine::default();
        m.create("g1".into(), "make tests pass", None, None, "t0")
            .unwrap();
        m
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            digest_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn create_and_unfinished() {
        let mut m = machine();
        assert!(matches!(
            m.create("g2".into(), "x", None, None, "t"),
            Err(Ok(_))
        ));
        assert!(matches!(
            Machine::default().create("x".into(), &"a".repeat(501), None, None, "t"),
            Err(Err(_))
        ));
        m.clear("t1");
        assert_eq!(
            m.goal.as_ref().unwrap().reason.as_deref(),
            Some("complete(user_requested)")
        );
        assert!(
            m.create("g3".into(), "next", Some(5000), Some(500), "t2")
                .is_ok()
        );
        assert_eq!(m.goal.as_ref().unwrap().max_turns, 200);
    }

    #[test]
    fn settle_transitions() {
        let mut m = machine();
        let g = m
            .settle(
                &Settlement {
                    made_progress: Some(true),
                    tokens_now: Some(100),
                    ..Settlement::default()
                },
                "t1",
            )
            .unwrap();
        assert_eq!((g.turn, g.tokens_at_start, g.revision), (1, 100, 1));
        let g = m
            .settle(
                &Settlement {
                    verdict: Some("not_met".into()),
                    ..Settlement::default()
                },
                "t2",
            )
            .unwrap();
        assert_eq!((g.status, g.not_met_streak), (Status::Active, 1));
        m.settle(
            &Settlement {
                verdict: Some("inconclusive".into()),
                ..Settlement::default()
            },
            "t3",
        );
        let g = m
            .settle(
                &Settlement {
                    verdict: Some("inconclusive".into()),
                    ..Settlement::default()
                },
                "t4",
            )
            .unwrap();
        assert_eq!(g.reason.as_deref(), Some("paused(verifier_inconclusive)"));
        assert!(
            m.settle(&Settlement::default(), "t5").is_none(),
            "paused goals do not settle"
        );
        m.resume("t6");
        let g = m
            .settle(
                &Settlement {
                    verdict: Some("met".into()),
                    ..Settlement::default()
                },
                "t7",
            )
            .unwrap();
        assert_eq!(g.status, Status::Complete);

        let mut b = machine();
        b.settle(
            &Settlement {
                blocked_proposal: true,
                ..Settlement::default()
            },
            "t",
        );
        b.settle(
            &Settlement {
                blocked_proposal: true,
                ..Settlement::default()
            },
            "t",
        );
        assert_eq!(
            b.settle(
                &Settlement {
                    blocked_proposal: true,
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .reason
            .as_deref(),
            Some("blocked(threshold_3turns)")
        );
        let mut s = machine();
        assert_eq!(
            s.settle(
                &Settlement {
                    safety_refusal: true,
                    blocked_proposal: true,
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .reason
            .as_deref(),
            Some("blocked(safety_policy)")
        );
        let mut t = Machine::default();
        t.create("x".into(), "o", Some(1000), None, "t").unwrap();
        t.settle(
            &Settlement {
                tokens_now: Some(10),
                ..Settlement::default()
            },
            "t",
        );
        assert_eq!(
            t.settle(
                &Settlement {
                    tokens_now: Some(1010),
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .reason
            .as_deref(),
            Some("budget_limited(token)")
        );
        let mut st = machine();
        for _ in 0..7 {
            st.settle(
                &Settlement {
                    made_progress: Some(false),
                    ..Settlement::default()
                },
                "t",
            );
        }
        assert_eq!(
            st.settle(
                &Settlement {
                    made_progress: Some(false),
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .reason
            .as_deref(),
            Some("paused(no_progress)")
        );
        let mut w = machine();
        assert_eq!(
            w.settle(
                &Settlement {
                    waiting: true,
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .status,
            Status::Waiting
        );
        assert_eq!(w.wake("t").unwrap().status, Status::Active);
        let mut u = machine();
        assert_eq!(
            u.settle(
                &Settlement {
                    verifier_unavailable: true,
                    verdict: Some("inconclusive".into()),
                    ..Settlement::default()
                },
                "t"
            )
            .unwrap()
            .reason
            .as_deref(),
            Some("paused(verifier_unavailable)")
        );
    }

    #[test]
    fn prompts_and_verifier() {
        assert!(kickoff("a<b").contains("<objective>\na&lt;b\n</objective>"));
        let g = machine().goal.unwrap();
        assert_eq!(
            continuation_hint(&g, Some("close")),
            format!(
                "{CONTINUATION_HINT}\nEvaluation: close\nGoal: \"make tests pass\" (turn 0/50)"
            )
        );
        assert_eq!(select_hint(&g, 0, false), Hint::NoTool);
        assert_eq!(select_hint(&g, 1, true), Hint::Recovery);
        assert_eq!(wait_backoff_ms(1), 5000);
        assert_eq!(wait_backoff_ms(3), 20_000);
        assert_eq!(wait_backoff_ms(30), 300_000);
        let v = parse_verification(
            "noise {\"verdict\": \"not_met\", \"reason\": \"tests fail\", \"missing\": [\"run tests\"]} trailing",
        );
        assert_eq!(
            (v.verdict.as_str(), v.missing.len(), v.failed),
            ("not_met", 1, false)
        );
        assert!(parse_verification("nothing").failed);
        assert!(parse_verification("{\"verdict\": \"maybe\"}").failed);
        let brief = evidence_brief(
            "o",
            "d",
            &Evidence {
                claim: Some("done".into()),
                files: vec!["a.rs".into()],
                ..Evidence::default()
            },
        );
        assert!(brief.contains("--- WORKER CLAIM (untrusted) ---\ndone\n--- CHANGED FILES ---\na.rs\n--- COMMANDS RUN ---\n(none reported)"));
        assert!(verification_prompt(&brief).starts_with("You are the completion verifier"));
        assert!(task_reminder(&["a  b".into()]).ends_with("\n- a b"));
        assert!(
            get_goal_json(&g, Some("completion"))
                .ends_with(" Pending proposal: completion (verification pending).")
        );
    }
}
