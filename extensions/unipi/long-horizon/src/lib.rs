//! `unipi.long-horizon` — goal / ralph / swarm / graph owners (unipi `packages/long-horizon`,
//! unipi-port §3.14).
//!
//! | unipi | here |
//! |---|---|
//! | gate at `before_agent_start` (child → explicit → owner → jev judge → default), mode section, tool exposure | [`Lh::gate`]; `set-active-tools` (no payload surgery, unipi-port §2.5); `tool_call` blocks hidden mode tools |
//! | owner coordinator (`state.json`, leases, history 10) | [`modes::Owners`] in `/ext-workspace/<wsid>/state.json` |
//! | goal machine, verifier, continuation, task gate, wait backoff, recovery outbox (`goal.json`) | [`goal`]; verifier through `models.complete` |
//! | ralph task files `.unipi/ralph/<name>.md` | `.oino/ralph/<name>.md` (+ `.state.json`) ([`ralph`]) |
//! | agent graph (`graph.json`, history 20) over the subagents graph-run port | [`graph`]; service `subagents.graph-run`, bus `unipi:subagents:run-done` |
//! | runaway guard (6 detectors, one steer per turn) | [`ralph::detect`] |
//! | nudge provider (priority 100) with a one-slot stash | arbiter `propose-nudge` |
//! | compaction brief / evidence | services `compaction-context.provide`, fan-out `evidence.contribute` |
//! | tools (schemas exact) | [`tools`] |
//! | `/goal`, `/ralph`, `/swarm`, `/graph`, `/regular`, `/visualize-progress`, runners | commands (+ hidden runners) |
//! | `LH_STATE`, `LH_PROGRESS`, owner/mode/ralph bus events, status slots | sticky topics, events, `setStatus` |

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests assert by unwrapping"
    )
)]
#![allow(
    clippy::too_many_lines,
    reason = "handlers keep unipi's function shapes one to one so the port stays auditable"
)]
#![allow(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "short local names mirror the unipi variables they port"
)]
#![allow(
    clippy::single_match_else,
    reason = "explicit Some/None arms mirror unipi's branches"
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

pub mod goal;
pub mod graph;
pub mod modes;
pub mod ralph;
pub mod tools;

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::session::Provenance;
use unipi_kit::services::{CompactionContext, fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::topics;
use unipi_kit::{fanout, jev};

use crate::goal::{Evidence, Hint, Machine, Settlement};
use crate::graph::{AddWork, GraphFile, GraphMode, Selected, Update};
use crate::modes::{EXTENSION_ID, Mode, NAMESPACE, OwnerKind, Owners, Settings};
use crate::ralph::{LoopState, RALPH_DIR, RALPH_GUEST_DIR};

const WAIT_TIMER: &str = "lh-goal-wait";
const GRAPH_WAKE_TIMER: &str = "lh-graph-wake";
const GRAPH_RETRY_TIMER: &str = "lh-graph-retry";
const VERIFIER_TIMEOUT_MS: u64 = 30_000;
const REMIND_AFTER: usize = 3;

/// The stashed continuation (one slot).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stash {
    text: String,
    title: String,
}

#[derive(Default)]
struct Turn {
    tool_calls: u32,
    commands: Vec<String>,
    files: Vec<String>,
    steps: Vec<ralph::Step>,
    steered: bool,
}

#[derive(Default)]
struct State {
    owners: Owners,
    machine: Machine,
    graph: GraphFile,
    ralph: Option<LoopState>,
    todos: Vec<ralph::Todo>,
    stash: Option<Stash>,
    pending_explicit: Option<Mode>,
    session_override: Option<Mode>,
    display: Option<Mode>,
    last_badge: Option<Mode>,
    turn_mode: Option<(Mode, String)>,
    turn: Turn,
    proposal: Option<(String, Option<String>, bool)>,
    waits: u32,
    verifier_reason: Option<String>,
    recovery_armed: bool,
    reminders: std::collections::HashMap<String, u32>,
    reported_attention: Vec<String>,
    judge_cache: Option<(String, Mode, f64, i64)>,
    loaded: bool,
}

/// The extension.
#[derive(Default)]
pub struct Lh {
    settings: Store,
    state: RefCell<State>,
}

fn now_iso() -> String {
    unipi_kit::fmt::iso_from_ms(timers::now_ms())
}

fn uuid() -> String {
    let h = unipi_kit::fmt::random_hex(32);
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

fn ws(rel: &str) -> PathBuf {
    unipi_kit::paths::workspace(rel)
}

fn text(t: impl Into<String>) -> ToolResult {
    ToolResult::text(t)
}

fn err(t: impl Into<String>) -> ToolResult {
    ToolResult::error(t)
}

fn task_path(name: &str) -> String {
    format!("{RALPH_DIR}/{}.md", ralph::sanitize(name))
}

fn task_guest(name: &str) -> PathBuf {
    PathBuf::from(RALPH_GUEST_DIR).join(format!("{}.md", ralph::sanitize(name)))
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

impl Lh {
    fn settings(&self) -> Settings {
        Settings::load(&self.settings.get(NAMESPACE))
    }

    // ── persistence ────────────────────────────────────────────────────────

    fn load(&self) {
        let mut st = self.state.borrow_mut();
        if st.loaded {
            return;
        }
        st.loaded = true;
        st.owners = unipi_kit::paths::read_json(&ws("state.json")).unwrap_or_default();
        st.machine.goal = unipi_kit::paths::read_json::<Value>(&ws("goal.json"))
            .and_then(|v| serde_json::from_value(v.get("goal")?.clone()).ok());
        st.graph = unipi_kit::paths::read_json(&ws("graph.json")).unwrap_or_default();
        let ralph_owner = st
            .owners
            .active
            .iter()
            .chain(&st.owners.parked)
            .find(|o| o.kind == OwnerKind::RalphLoop)
            .map(|o| o.label.clone());
        st.ralph = ralph_owner.and_then(|n| {
            unipi_kit::paths::read_json(
                &PathBuf::from(RALPH_GUEST_DIR).join(format!("{}.state.json", ralph::sanitize(&n))),
            )
        });
    }

    fn save(&self) {
        let st = self.state.borrow();
        let _ = unipi_kit::paths::write_json(&ws("state.json"), &st.owners);
        let _ = unipi_kit::paths::write_json(
            &ws("goal.json"),
            &json!({"version": 1, "goal": st.machine.goal}),
        );
        let _ = unipi_kit::paths::write_json(
            &ws("graph.json"),
            &json!({"v": 1, "current": st.graph.current, "history": st.graph.history}),
        );
        if let Some(l) = &st.ralph {
            let _ = unipi_kit::paths::write_json(
                &PathBuf::from(RALPH_GUEST_DIR).join(format!("{}.state.json", l.name)),
                l,
            );
        }
        drop(st);
        self.publish();
    }

    fn publish(&self) {
        let st = self.state.borrow();
        let mode = st
            .owners
            .active
            .as_ref()
            .map_or_else(|| st.display.unwrap_or(Mode::None), |o| o.kind.mode());
        let mut lh = json!({"mode": mode.as_str()});
        if let Some(p) = &st.owners.parked {
            lh["paused"] = json!(p.kind.mode().as_str());
        }
        if let Some(o) = st.owners.active.as_ref().or(st.owners.parked.as_ref()) {
            lh["owner"] = json!({"kind": o.kind.as_str(), "status": if o.status == modes::OwnerStatus::Active { "active" } else { "parked" }});
        }
        if let Some((k, at)) = &st.owners.last_stop {
            lh["lastStop"] = json!({"kind": k, "at": at});
        }
        unipi_kit::bus::emit_raw(topics::LH_STATE, &lh);
        let ralph_progress = st.ralph.as_ref().map(|l| {
            let (c, t, _) = ralph::progress(
                &std::fs::read_to_string(task_guest(&l.name)).unwrap_or_default(),
                l.items_per_iteration,
            );
            json!({"name": l.name, "iteration": l.iteration, "checked": c, "total": t})
        });
        unipi_kit::bus::emit_raw(
            topics::LH_PROGRESS,
            &json!({
                "mode": mode.as_str(),
                "goal": st.machine.goal.as_ref().map(|g| json!({"objective": g.objective, "status": g.status.as_str(), "turn": g.turn, "maxTurns": g.max_turns})),
                "ralph": ralph_progress,
                "graph": st.graph.current.as_ref().map(|g| json!({"graphId": g.graph_id, "mode": g.mode, "task": g.task, "counts": st.graph.counts(g)})),
                "todos": ralph::todo_line(&st.todos),
            }),
        );
        let label = match mode {
            Mode::None => None,
            m => Some(match &st.owners.parked {
                Some(p) if st.owners.active.is_none() => {
                    format!("{} · paused", p.kind.mode().label())
                }
                _ => m.label().to_owned(),
            }),
        };
        let _ = ui::status("lh-mode", label.as_deref());
        let goal = st
            .machine
            .active()
            .map(|g| format!("◎ goal {}/{}", g.turn, g.max_turns));
        let _ = ui::status("lh-goal", goal.as_deref());
    }

    fn owner_event(event: &str, owner: Option<&modes::Owner>, reason: Option<&str>) {
        unipi_kit::bus::emit(&topics::LhOwnerChanged {
            event: event.into(),
            owner_id: owner.map(|o| o.owner_id.clone()),
            kind: owner.map(|o| o.kind.as_str().to_owned()),
            status: None,
            reason: reason.map(ToOwned::to_owned),
        });
    }

    // ── delivery ───────────────────────────────────────────────────────────

    /// Stashes a continuation for the next settle (or sends it now when idle).
    async fn send(&self, message: &str, title: &str) {
        if control::is_idle() {
            Self::send_now(message, title).await;
            self.state.borrow_mut().machine.clear_pending();
        } else {
            self.state.borrow_mut().stash = Some(Stash {
                text: message.to_owned(),
                title: title.to_owned(),
            });
        }
    }

    async fn send_now(message: &str, title: &str) {
        let prov = Provenance {
            source: "Long-horizon".into(),
            title: Some(title.to_owned()),
            synopsis: None,
            severity: None,
        };
        let opts = oino_extension::wit::session::UserMessageOptions {
            deliver_as: (!control::is_idle()).then_some(session::Delivery::FollowUp),
            expand_prompt_templates: false,
            provenance: Some(prov),
        };
        let _ =
            oino_extension::wit::session::send_user_message(json!(message).to_string(), opts).await;
    }

    fn deliver(&self, message: &str, persist: bool) -> String {
        if persist {
            self.state.borrow_mut().machine.record_pending(message, 0);
        }
        message.to_owned()
    }

    // ── gate ───────────────────────────────────────────────────────────────

    async fn judge(&self, prompt: &str) -> Option<(Mode, f64)> {
        let s = self.settings();
        if !s.judge_enabled {
            return None;
        }
        let hash = goal::digest_hex(prompt.as_bytes());
        let now = timers::now_ms();
        if let Some((h, m, c, at)) = self.state.borrow().judge_cache.clone()
            && h == hash
            && now - at < 10_000
        {
            return Some((m, c));
        }
        let mut all = control::settings()
            .ok()
            .and_then(|v| v.get("extensionSettings").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        all[NAMESPACE] = self.settings.get(NAMESPACE).0;
        let js = jev::Settings::resolve(&all, Some(NAMESPACE));
        let answers = jev::ask(&js, prompt, &modes::judge_questions()).await?;
        let a = answers.get("mode")?;
        let mode = Mode::parse(a.choice.as_deref()?)?;
        let conf = a
            .confidence
            .filter(|c| (0.0..=1.0).contains(c))
            .unwrap_or(0.0);
        self.state.borrow_mut().judge_cache = Some((hash, mode, conf, now));
        Some((mode, conf))
    }

    async fn gate(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        self.load();
        let prompt = ev
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let (mode, source, confidence) = if cx.is_child() {
            self.state.borrow_mut().pending_explicit = None;
            (Mode::None, "child", None)
        } else {
            let (fresh, sticky) = {
                let st = self.state.borrow();
                (st.pending_explicit, st.session_override)
            };
            let mut explicit = fresh.or(sticky);
            if let Some(m) = explicit {
                let switch = self
                    .state
                    .borrow()
                    .owners
                    .active
                    .as_ref()
                    .is_some_and(|o| o.kind.mode() != m);
                if switch {
                    let ok = self
                        .state
                        .borrow_mut()
                        .owners
                        .suspend(&format!("paused(superseded_by:{})", m.as_str()), &now_iso())
                        .is_some();
                    if !ok {
                        explicit = None;
                    }
                }
            }
            let owner = self
                .state
                .borrow()
                .owners
                .active
                .as_ref()
                .map(|o| o.kind.mode());
            let r = if let Some(m) = explicit {
                (m, "explicit", None)
            } else if let Some(m) = owner {
                (m, "owner", None)
            } else if let Some((m, c)) = self.judge(&prompt).await {
                let s = self.settings();
                if c >= s.judge_threshold {
                    (m, "judge", Some(c))
                } else {
                    (s.default_mode, "judge_abstained_low_confidence", Some(c))
                }
            } else {
                (self.settings().default_mode, "default", None)
            };
            self.state.borrow_mut().pending_explicit = None;
            let sticky_badge = fresh.is_none() && sticky.is_some() && explicit == sticky;
            if sticky_badge { (r.0, r.1, r.2) } else { r }
        };
        let tools = control::active_tools();
        if let Some(next) = modes::sync_mode_tools(&tools, mode) {
            let _ = control::set_active_tools(&next);
        }
        let show_badge = {
            let mut st = self.state.borrow_mut();
            st.display = Some(mode);
            st.turn_mode = Some((mode, source.to_owned()));
            let show = source == "explicit" || st.last_badge != Some(mode);
            st.last_badge = Some(mode);
            st.turn = Turn::default();
            show
        };
        let mut resolved = json!({"mode": mode.as_str(), "source": source});
        if let Some(c) = confidence {
            resolved["confidence"] = json!(c);
        }
        unipi_kit::bus::emit_raw(topics::LONG_HORIZON_MODE_RESOLVED, &resolved);
        if show_badge {
            let _ = session::append_entry("long-horizon-decision", Some(&resolved));
        }
        self.publish();
        let fragment = {
            let st = self.state.borrow();
            modes::mode_fragment(mode, st.owners.active.as_ref(), st.owners.parked.as_ref())
        };
        let mut opts = ev
            .get("systemPromptOptions")
            .cloned()
            .filter(Value::is_object)?;
        if !opts.get("sections").is_some_and(Value::is_object) {
            opts["sections"] = json!({});
        }
        opts["sections"]["long-horizon"] = json!(fragment);
        Some(json!({"systemPromptOptions": opts}))
    }

    fn tool_call(&self, ev: &Event) -> Option<Value> {
        let name = ev
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = ev.get("input").cloned().unwrap_or(Value::Null);
        let mut st = self.state.borrow_mut();
        let (mode, _) = st.turn_mode.clone()?;
        if modes::hidden_tools(mode).contains(&name) {
            return Some(
                json!({"block": true, "reason": format!("long-horizon: \"{name}\" is not available in {} mode. Switch with /goal, /ralph, /swarm, or /graph.", mode.as_str())}),
            );
        }
        if mode == Mode::None {
            return None;
        }
        st.turn.tool_calls += 1;
        if let Some(c) = input.get("command").and_then(Value::as_str) {
            st.turn.commands.push(c.chars().take(200).collect());
        }
        if matches!(name, "edit" | "write")
            && let Some(p) = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .and_then(Value::as_str)
        {
            st.turn.files.push(p.chars().take(200).collect());
        }
        None
    }

    async fn tool_result(&self, ev: &Event) {
        let step = ralph::Step {
            tool: ev
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            input: ralph::stable(ev.get("input").unwrap_or(&Value::Null)),
            result: ev
                .get("content")
                .and_then(Value::as_array)
                .and_then(|c| c.iter().find_map(|p| p.get("text").and_then(Value::as_str)))
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect(),
            is_error: ev.get("isError").and_then(Value::as_bool).unwrap_or(false),
            command: ev
                .payload
                .pointer("/input/command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        };
        let reminder = {
            let mut st = self.state.borrow_mut();
            if st.owners.active.is_none() || st.turn.steered {
                return;
            }
            st.turn.steps.push(step);
            match ralph::detect(&st.turn.steps, REMIND_AFTER) {
                Some((sig, n)) => {
                    st.turn.steered = true;
                    Some(ralph::runaway_reminder(sig, n))
                }
                None => None,
            }
        };
        if let Some(r) = reminder {
            let _ = session::send_user_message(&json!(r), Some(session::Delivery::Steer)).await;
        }
    }

    // ── goal continuation ──────────────────────────────────────────────────

    async fn verify(&self, objective: &str, brief: String) -> goal::Verdict {
        let evidence = fanout::gather_evidence(fanout::DEFAULT_TIMEOUT).await;
        if !evidence.blocking.is_empty() {
            return goal::Verdict {
                verdict: "not_met".into(),
                reason: "board: open work remains".into(),
                missing: evidence.blocking,
                failed: false,
            };
        }
        let brief = if evidence.notes.is_empty() {
            brief
        } else {
            format!(
                "{brief}\n--- MODULE EVIDENCE ---\n{}",
                evidence.notes.join("\n")
            )
        };
        let _ = objective;
        let prompt = goal::verification_prompt(&brief);
        let model = self.settings().verifier_model;
        let mut req = json!({"context": {"messages": [{"role": "user", "content": [{"type": "text", "text": prompt}], "timestamp": timers::now_ms()}]}, "options": {}});
        if !model.is_empty() {
            req["model"] = json!(model);
        }
        let call = models::complete(&req);
        let timeout = timers::sleep(Duration::from_millis(VERIFIER_TIMEOUT_MS));
        futures::pin_mut!(call, timeout);
        match futures::future::select(call, timeout).await {
            futures::future::Either::Left((Ok(msg), _)) => {
                let t: String = msg
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                goal::parse_verification(&t)
            }
            futures::future::Either::Left((Err(_), _)) => {
                goal::Verdict::failure("Evaluator call failed (continuing)")
            }
            futures::future::Either::Right(_) => {
                goal::Verdict::failure("Evaluator timed out (continuing)")
            }
        }
    }

    fn tokens() -> Option<u64> {
        control::session_stats()
            .ok()?
            .pointer("/tokens/total")
            .and_then(Value::as_u64)
    }

    fn tail() -> Vec<(String, String)> {
        session::branch(None)
            .unwrap_or_default()
            .iter()
            .rev()
            .filter_map(|e| {
                let m = e.get("message")?;
                let role = m.get("role")?.as_str()?;
                if !matches!(role, "user" | "assistant") {
                    return None;
                }
                let t = match m.get("content")? {
                    Value::String(s) => s.clone(),
                    Value::Array(a) => a
                        .iter()
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => return None,
                };
                Some((role.to_owned(), t))
            })
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// unipi `GoalContinuation.onTurnEnd` (goal owner only).
    #[allow(clippy::too_many_lines, reason = "one continuation decision, as unipi")]
    async fn goal_turn_end(&self, aborted: bool) {
        let (goal, is_goal_owner) = {
            let st = self.state.borrow();
            (
                st.machine.active().cloned(),
                st.owners
                    .active
                    .as_ref()
                    .is_some_and(|o| o.kind == OwnerKind::Goal),
            )
        };
        let Some(g) = goal else {
            self.state.borrow_mut().proposal = None;
            return;
        };
        if !is_goal_owner {
            return;
        }
        if !g.kickoff_delivered {
            let k = self.deliver(&goal::kickoff(&g.objective), true);
            {
                let mut st = self.state.borrow_mut();
                st.machine.mark_kickoff(&now_iso());
                st.proposal = None;
                st.waits = 0;
                st.verifier_reason = None;
            }
            self.save();
            self.send(&k, "Kickoff").await;
            return;
        }
        let (proposal, activity) = {
            let mut st = self.state.borrow_mut();
            (
                st.proposal.take(),
                (
                    st.turn.tool_calls,
                    st.turn.files.clone(),
                    st.turn.commands.clone(),
                ),
            )
        };
        let tokens = Self::tokens();
        let todos: Vec<String> = self
            .state
            .borrow()
            .todos
            .iter()
            .filter(|t| matches!(t.status.as_str(), "pending" | "in_progress"))
            .map(|t| t.content.clone())
            .collect();
        let mut reminder = None;
        let settled = match proposal {
            Some((kind, summary, _)) if kind == "completion" => {
                let reminded = self
                    .state
                    .borrow()
                    .reminders
                    .get(&g.goal_id)
                    .copied()
                    .unwrap_or(0);
                if !todos.is_empty() && reminded < goal::TASK_REMINDERS_PER_GOAL {
                    reminder = Some(goal::task_reminder(&todos));
                    self.state.borrow_mut().machine.settle(
                        &Settlement {
                            made_progress: (activity.0 > 0).then_some(true),
                            tokens_now: tokens,
                            ..Settlement::default()
                        },
                        &now_iso(),
                    )
                } else {
                    let brief = goal::evidence_brief(
                        &g.objective,
                        &g.objective_digest,
                        &Evidence {
                            claim: summary.clone(),
                            files: activity.1.clone(),
                            commands: activity.2.clone(),
                            tail: Self::tail(),
                        },
                    );
                    let mut v = self.verify(&g.objective, brief.clone()).await;
                    if v.failed {
                        v = self.verify(&g.objective, brief).await;
                    }
                    self.state.borrow_mut().verifier_reason = (!v.failed).then(|| {
                        format!(
                            "{}{}",
                            v.reason,
                            if v.missing.is_empty() {
                                String::new()
                            } else {
                                format!(" (missing: {})", v.missing.join("; "))
                            }
                        )
                    });
                    let s = self.state.borrow_mut().machine.settle(
                        &Settlement {
                            verdict: Some(v.verdict.clone()),
                            verifier_unavailable: v.failed,
                            made_progress: (v.verdict == "not_met")
                                .then_some(!activity.1.is_empty()),
                            tokens_now: tokens,
                            ..Settlement::default()
                        },
                        &now_iso(),
                    );
                    if let Some(p) = s.as_ref().filter(|p| p.status == goal::Status::Paused) {
                        let msg = match p.reason.as_deref() {
                            Some("paused(verifier_unavailable)") => format!(
                                "Goal verifier unavailable twice ({}) — goal paused. /goal resume to retry.",
                                v.reason
                            ),
                            Some("paused(verifier_inconclusive)") => format!(
                                "Goal verifier returned inconclusive twice — goal paused. Last evaluation: {}. /goal resume.",
                                v.reason
                            ),
                            _ => format!(
                                "Completion claim rejected {} times — goal paused.{} /goal resume.",
                                p.not_met_streak,
                                if v.missing.is_empty() {
                                    String::new()
                                } else {
                                    format!(" Missing: {}.", v.missing.join("; "))
                                }
                            ),
                        };
                        let _ = ui::notify(&msg, "warning");
                    }
                    s
                }
            }
            Some((_, _, safety)) => self.state.borrow_mut().machine.settle(
                &Settlement {
                    blocked_proposal: true,
                    safety_refusal: safety,
                    tokens_now: tokens,
                    ..Settlement::default()
                },
                &now_iso(),
            ),
            None => self.state.borrow_mut().machine.settle(
                &Settlement {
                    made_progress: (activity.0 > 0).then_some(true),
                    tokens_now: tokens,
                    ..Settlement::default()
                },
                &now_iso(),
            ),
        };
        let Some(s) = settled else {
            self.save();
            return;
        };
        if !matches!(s.status, goal::Status::Active | goal::Status::Waiting) {
            let terminal = s
                .reason
                .clone()
                .unwrap_or_else(|| s.status.as_str().to_owned());
            let wrap =
                s.status == goal::Status::BudgetLimited && s.wrap_up_delivered_revision.is_none();
            {
                let mut st = self.state.borrow_mut();
                if wrap && let Some(gg) = st.machine.goal.as_mut() {
                    gg.wrap_up_delivered_revision = Some(s.revision);
                }
                let now = now_iso();
                let o = if s.status == goal::Status::Paused {
                    st.owners
                        .suspend(&terminal, &now)
                        .or_else(|| st.owners.finish(&terminal, &now, timers::now_ms()))
                } else {
                    st.owners.finish(&terminal, &now, timers::now_ms())
                };
                drop(st);
                Self::owner_event(
                    if s.status == goal::Status::Paused {
                        "suspended"
                    } else {
                        "finished"
                    },
                    o.as_ref(),
                    Some(&terminal),
                );
            }
            self.save();
            if wrap {
                self.send(goal::WRAP_UP_PROMPT, "Wrap-up").await;
            }
            return;
        }
        if s.status == goal::Status::Waiting {
            let waits = {
                let mut st = self.state.borrow_mut();
                if !aborted {
                    st.machine
                        .record_pending(&goal::continuation_hint(&s, None), 0);
                }
                st.waits += 1;
                st.waits
            };
            timers::set(
                WAIT_TIMER,
                Duration::from_millis(goal::wait_backoff_ms(waits)),
                false,
            );
            self.save();
            return;
        }
        let (hint, verifier_reason) = {
            let mut st = self.state.borrow_mut();
            st.waits = 0;
            let h = goal::select_hint(&s, activity.0, st.recovery_armed);
            st.recovery_armed = false;
            if aborted {
                st.machine.clear_pending();
            }
            (h, st.verifier_reason.clone())
        };
        let message = if let Some(r) = reminder {
            *self
                .state
                .borrow_mut()
                .reminders
                .entry(s.goal_id.clone())
                .or_default() += 1;
            let base = if hint == Hint::Recovery {
                format!(
                    "{}\n\n{}",
                    goal::RECOVERY_FRAGMENT,
                    goal::continuation_hint(&s, None)
                )
            } else {
                goal::continuation_hint(&s, None)
            };
            format!(
                "{base}\n\n{r}\n\nYour completion claim was not evaluated: reconcile the task list first, then call update_goal again if the objective is met."
            )
        } else {
            match hint {
                Hint::Recovery => goal::RECOVERY_FRAGMENT.to_owned(),
                Hint::NoTool => goal::NO_TOOL_NUDGE.to_owned(),
                Hint::NoProgress => goal::NO_PROGRESS_NUDGE.to_owned(),
                Hint::Audit => goal::TERMINAL_AUDIT.to_owned(),
                Hint::Hint => goal::continuation_hint(&s, verifier_reason.as_deref()),
            }
        };
        let m = self.deliver(&message, !aborted);
        self.save();
        self.state.borrow_mut().stash = Some(Stash {
            text: m,
            title: "Continuation".into(),
        });
    }

    // ── tools ──────────────────────────────────────────────────────────────

    async fn run_tool(&'static self, name: &str, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        self.load();
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(ToOwned::to_owned);
        let r = match name {
            "create_goal" => self.create_goal(
                &s("objective").unwrap_or_default(),
                args.get("token_budget").and_then(Value::as_u64),
                cx.cx.is_child(),
            ),
            "get_goal" => {
                let st = self.state.borrow();
                text(st.machine.goal.as_ref().map_or_else(
                    || "No goal exists for this session.".to_owned(),
                    |g| goal::get_goal_json(g, st.proposal.as_ref().map(|p| p.0.as_str())),
                ))
            }
            "update_goal" => self.update_goal(args),
            "todowrite" => match ralph::validate_todos(args.get("todos").unwrap_or(&Value::Null)) {
                Ok(list) => {
                    let line = ralph::todo_line(&list);
                    self.state.borrow_mut().todos = list;
                    self.publish();
                    text(format!("Plan updated: {line}"))
                }
                Err(e) => text(format!("Todo list rejected: {e}")),
            },
            "ralph_done" => self.ralph_done().await,
            "loop_status" => {
                let st = self.state.borrow();
                match &st.ralph {
                    None => text("No loop state in this session."),
                    Some(l) => {
                        let (c, t, next) = ralph::progress(
                            &std::fs::read_to_string(task_guest(&l.name)).unwrap_or_default(),
                            l.items_per_iteration,
                        );
                        let mut v = json!({"name": l.name, "iteration": l.iteration});
                        if l.max_iterations > 0 {
                            v["max_iterations"] = json!(l.max_iterations);
                        }
                        for (k, x) in [
                            ("items_per_iteration", json!(l.items_per_iteration)),
                            ("reflect_every", json!(l.reflect_every)),
                            ("status", json!(l.status)),
                            ("task_file", json!(task_path(&l.name))),
                            ("progress", json!({"checked": c, "total": t, "next": next})),
                        ] {
                            v[k] = x;
                        }
                        text(serde_json::to_string_pretty(&v).unwrap_or_default())
                    }
                }
            }
            "update_agent_graph" => self.update_graph(args, cx).await,
            "view_agent_graph" => {
                let st = self.state.borrow();
                let history = args.get("history").and_then(Value::as_bool) == Some(true);
                let g = match s("graph_id") {
                    Some(id) => st.graph.graph(&id),
                    None => st.graph.current.as_ref(),
                };
                match g {
                    Some(g) => text(serde_json::to_string_pretty(&st.graph.view(g, history)).unwrap_or_default()),
                    None if history && !st.graph.history.is_empty() => text(serde_json::to_string_pretty(&json!({"current": null, "history": st.graph.history_json()})).unwrap_or_default()),
                    None => text(s("graph_id").map_or_else(|| "No agent graph yet — schedule work with update_agent_graph operation=add_work.".to_owned(), |id| format!("No graph {id}."))),
                }
            }
            "yield_agent_graph" => {
                let st = self.state.borrow();
                match st.graph.open() {
                    None => err(
                        "No open agent graph — nothing to yield for. Finish your answer instead.",
                    ),
                    Some(g) => {
                        let running = g.work.iter().filter(|w| w.status == "running").count();
                        let queued = g
                            .work
                            .iter()
                            .filter(|w| st.graph.view_status(g, w) == "queued")
                            .count();
                        if running == 0 && queued == 0 {
                            err(
                                "Agent graph has no in-flight or pending work that can produce a future checkpoint; inspect results and finish or update the graph instead.",
                            )
                        } else {
                            text(format!("Yielded: {running} running, {queued} waiting. You will be woken at the next checkpoint.")).terminate()
                        }
                    }
                }
            }
            "agent_output" => {
                let st = self.state.borrow();
                let id = s("id").unwrap_or_default();
                match st.graph.read_result(id.trim(), s("graph_id").as_deref()) {
                    None => err(format!("No graph work or result {id}.")),
                    Some((gid, w)) => {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "clamped"
                        )]
                        let max = args
                            .get("max_bytes")
                            .and_then(Value::as_f64)
                            .unwrap_or(32_768.0)
                            .clamp(1024.0, 131_072.0) as usize;
                        let head = format!(
                            "work {} · {}{}{} · graph {gid}",
                            w.work_id,
                            w.status,
                            w.result_id
                                .as_ref()
                                .map(|r| format!(" · result {r}"))
                                .unwrap_or_default(),
                            w.operator_id
                                .as_ref()
                                .map(|o| format!(" · operator {o}"))
                                .unwrap_or_default()
                        );
                        if w.status == "completed" || w.result.is_some() {
                            let t = w.result.clone().unwrap_or_default();
                            let body = if t.len() > max {
                                format!(
                                    "{}\n[…truncated: {} bytes]",
                                    graph::truncate_utf8(&t, max),
                                    t.len()
                                )
                            } else {
                                t
                            };
                            text(format!(
                                "{head}{}\n\n{body}",
                                if w.result_truncated {
                                    " · stored text truncated"
                                } else {
                                    ""
                                }
                            ))
                        } else if w.status == "failed" {
                            text(format!(
                                "{head}\nFailure: {}",
                                w.failure.unwrap_or_else(|| "unknown".into())
                            ))
                        } else {
                            text(format!("{head}\nNo committed result yet."))
                        }
                    }
                }
            }
            "agent_swarm_status" => {
                let st = self.state.borrow();
                st.graph.current.as_ref().map_or_else(|| text("No swarm yet — schedule items with update_agent_graph operation=add_work."), |g| text(serde_json::to_string_pretty(&st.graph.swarm_status(g)).unwrap_or_default()))
            }
            _ => err(format!("unknown tool {name}")),
        };
        self.save();
        Ok(r)
    }

    fn create_goal(&self, objective: &str, budget: Option<u64>, child: bool) -> ToolResult {
        if child {
            return err(modes::CHILD_OWNER_REFUSAL);
        }
        let mut st = self.state.borrow_mut();
        if let Some(g) = st
            .machine
            .goal
            .as_ref()
            .filter(|g| g.status == goal::Status::Paused)
        {
            return text(format!(
                "Goal not set: a parked goal exists (\"{}\", {}). Ask the user to /goal resume or /goal clear first.",
                g.objective,
                g.status.as_str()
            ));
        }
        let g = match st
            .machine
            .create(uuid(), objective, budget, None, &now_iso())
        {
            Ok(g) => g,
            Err(Ok(g)) => {
                return text(format!(
                    "Goal not set: unfinished goal \"{}\" is {}. It must complete (or be cleared) before another goal starts.",
                    g.objective,
                    g.status.as_str()
                ));
            }
            Err(Err(e)) => return err(e),
        };
        match st.owners.active.as_ref().map(|o| o.kind) {
            None => {
                let o = st
                    .owners
                    .activate(uuid(), OwnerKind::Goal, &g.objective, &now_iso());
                drop(st);
                Self::owner_event("activated", o.as_ref(), None);
                st = self.state.borrow_mut();
            }
            Some(OwnerKind::Goal) => {}
            Some(k) => {
                return text(format!(
                    "Goal recorded but not driving: session is owned by a {} owner. Finish or suspend it first.",
                    k.as_str()
                ));
            }
        }
        st.proposal = None;
        let limits = [
            Some(format!("max {} turns", g.max_turns)),
            Some(format!("stall after {} no-progress turns", g.stall_cap)),
            g.token_budget.map(|b| format!("budget {b} tokens")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        text(format!(
            "Goal set: \"{}\" ({limits}). The host will verify any completion claim with an independent evaluator before the goal completes.",
            g.objective
        ))
    }

    fn update_goal(&self, args: &Value) -> ToolResult {
        let mut st = self.state.borrow_mut();
        let Some(g) = st.machine.goal.clone() else {
            return text("No goal to update.");
        };
        if !matches!(g.status, goal::Status::Active | goal::Status::Waiting) {
            return text(format!(
                "Goal is {} — terminal proposals are only accepted while active.",
                g.status.as_str()
            ));
        }
        let s = |k: &str| args.get(k).and_then(Value::as_str);
        if s("mode") == Some("token_budget") {
            if s("expected_goal_id") != Some(g.goal_id.as_str())
                || s("expected_updated_at") != Some(g.updated_at.as_str())
            {
                return text(
                    "Budget not updated: expected_goal_id/expected_updated_at do not match the current goal. Call get_goal again and retry with fresh values.",
                );
            }
            let budget = args.get("token_budget").and_then(Value::as_u64);
            if budget.is_some_and(|b| b < goal::TOKEN_BUDGET_MINIMUM) {
                return text(format!(
                    "Budget not updated: minimum is {} tokens.",
                    goal::TOKEN_BUDGET_MINIMUM
                ));
            }
            st.machine.set_budget(budget, &now_iso());
            return text(budget.map_or_else(|| "Token budget cleared; the goal runs to its turn/stall limits.".to_owned(), |b| format!("Token budget set to {b}. A budget_limited stop fires when cumulative tokens reach it.")));
        }
        let Some(status) = s("status") else {
            return text("Missing status for mode \"status\".");
        };
        if let Some((k, _, _)) = &st.proposal {
            return text(format!(
                "A {k} proposal is already pending verification this turn."
            ));
        }
        let safety = args.get("safety_refusal").and_then(Value::as_bool) == Some(true);
        st.proposal = Some((
            if status == "complete" {
                "completion"
            } else {
                "blocked"
            }
            .into(),
            s("summary").map(ToOwned::to_owned),
            safety,
        ));
        if status == "complete" {
            text(
                "Completion proposed. The independent verifier will judge the evidence at turn end. If the verdict is not_met, the missing evidence will be reported — keep working; repeated failed claims count as no progress.",
            )
        } else if safety {
            text("Safety refusal recorded; the host will terminal-block this goal immediately.")
        } else {
            text(
                "Blocked proposal recorded. The host blocks the goal only after the same blocker repeats for 3 consecutive turns.",
            )
        }
    }

    // ── ralph ──────────────────────────────────────────────────────────────

    fn ralph_start(&self, name: &str, content: &str) -> std::result::Result<String, String> {
        let mut st = self.state.borrow_mut();
        if let Some(a) = &st.owners.active {
            return Err(format!(
                "session is owned by a {} owner — finish or suspend it first",
                a.kind.as_str()
            ));
        }
        let items = ralph::parse_checklist(content);
        if items.is_empty() {
            return Err("task content has no checklist items (`- [ ]`)".into());
        }
        let g = match st.machine.create(
            uuid(),
            &format!("Complete every checklist item in {}", task_path(name)),
            None,
            None,
            &now_iso(),
        ) {
            Ok(g) => g,
            Err(Ok(g)) => {
                return Err(format!(
                    "unfinished goal exists ({}) — clear or complete it first",
                    g.status.as_str()
                ));
            }
            Err(Err(e)) => return Err(e),
        };
        let path = task_guest(name);
        unipi_kit::paths::ensure_parent(&path)
            .and_then(|()| std::fs::write(&path, content))
            .map_err(|e| format!("{}: {e}", task_path(name)))?;
        let state = LoopState {
            version: 1,
            loop_id: uuid(),
            name: ralph::sanitize(name),
            task_file: format!("{}.md", ralph::sanitize(name)),
            iteration: 1,
            max_iterations: 0,
            items_per_iteration: 2,
            reflect_every: 5,
            status: "active".into(),
            goal_id: g.goal_id,
            started_at: now_iso(),
        };
        let prompt = ralph::iteration_prompt(&state, content, &task_path(name), false);
        let o = st
            .owners
            .activate(uuid(), OwnerKind::RalphLoop, name, &now_iso());
        st.machine.mark_kickoff(&now_iso());
        st.machine.record_pending(&prompt, 0);
        st.ralph = Some(state.clone());
        st.session_override = Some(Mode::Ralph);
        drop(st);
        Self::owner_event("activated", o.as_ref(), None);
        unipi_kit::bus::emit_raw(
            topics::RALPH_ITERATION_DONE,
            &json!({"name": state.name, "iteration": 0, "remaining": items.len()}),
        );
        Ok(prompt)
    }

    async fn ralph_done(&self) -> ToolResult {
        let (state, owner_ok) = {
            let st = self.state.borrow();
            (
                st.ralph.clone(),
                st.owners
                    .active
                    .as_ref()
                    .is_some_and(|o| o.kind == OwnerKind::RalphLoop),
            )
        };
        let Some(state) = state else {
            return text("ralph_done rejected: no active loop");
        };
        if !owner_ok {
            return text("ralph_done rejected: ralph is not the active owner (suspended?)");
        }
        let md = std::fs::read_to_string(task_guest(&state.name)).unwrap_or_default();
        let remaining = ralph::parse_checklist(&md)
            .iter()
            .filter(|i| !i.checked)
            .count();
        self.state.borrow_mut().machine.settle(
            &Settlement {
                made_progress: Some(true),
                ..Settlement::default()
            },
            &now_iso(),
        );
        if remaining == 0 {
            unipi_kit::bus::emit(&topics::RalphIterationDone {
                name: state.name.clone(),
                iteration: state.iteration,
                remaining: 0,
            });
            let objective = self
                .state
                .borrow()
                .machine
                .active()
                .map(|g| (g.objective.clone(), g.objective_digest.clone()));
            let Some((obj, digest)) = objective else {
                return text(
                    "Verifier was inconclusive (no active loop goal); the loop continues next iteration.",
                );
            };
            let brief = goal::evidence_brief(
                &obj,
                &digest,
                &Evidence {
                    claim: Some("Every checklist item in the task file is checked.".into()),
                    files: vec![task_path(&state.name)],
                    ..Evidence::default()
                },
            );
            let v = self.verify(&obj, brief).await;
            let settled = self.state.borrow_mut().machine.settle(
                &Settlement {
                    verdict: Some(v.verdict.clone()),
                    ..Settlement::default()
                },
                &now_iso(),
            );
            if v.verdict == "met" || settled.is_some_and(|g| g.status == goal::Status::Complete) {
                let o = {
                    let mut st = self.state.borrow_mut();
                    if let Some(l) = st.ralph.as_mut() {
                        l.status = "complete".into();
                    }
                    st.session_override = None;
                    st.owners
                        .finish("complete(verifier_met)", &now_iso(), timers::now_ms())
                };
                Self::owner_event("finished", o.as_ref(), Some("complete(verifier_met)"));
                unipi_kit::bus::emit(&topics::RalphLoopEnd {
                    name: state.name.clone(),
                    reason: "complete(verifier_met)".into(),
                    iterations: state.iteration,
                });
                let _ = ui::notify(
                    &format!(
                        "Ralph loop \"{}\" complete after {} iterations.",
                        state.name, state.iteration
                    ),
                    "info",
                );
                return text(format!(
                    "Loop complete — the verifier accepted the checklist ({}). Report the final summary to the user.",
                    v.reason
                ));
            }
            if v.verdict == "not_met" {
                return text(format!(
                    "Verifier rejected the completion claim: {}{}\nUncheck or finish the remaining work; the next iteration continues.",
                    v.reason,
                    if v.missing.is_empty() {
                        String::new()
                    } else {
                        format!("\nMissing evidence: {}", v.missing.join("; "))
                    }
                ));
            }
            return text(format!(
                "Verifier was inconclusive ({}); the loop continues next iteration.",
                v.reason
            ));
        }
        let next = LoopState {
            iteration: state.iteration + 1,
            ..state
        };
        let reflection =
            next.reflect_every > 0 && next.iteration.is_multiple_of(next.reflect_every);
        let prompt = ralph::iteration_prompt(&next, &md, &task_path(&next.name), reflection);
        {
            let mut st = self.state.borrow_mut();
            st.ralph = Some(next.clone());
            st.machine.record_pending(&prompt, 0);
            st.stash = Some(Stash {
                text: prompt,
                title: "Ralph".into(),
            });
        }
        unipi_kit::bus::emit(&topics::RalphIterationDone {
            name: next.name.clone(),
            iteration: next.iteration,
            remaining: u32::try_from(remaining).unwrap_or(u32::MAX),
        });
        text(format!(
            "Iteration yielded. Next iteration dispatched. When every item is checked and verified, the final ralph_done completes the loop; do not emit {} manually.",
            ralph::RALPH_COMPLETE_MARKER
        ))
    }

    // ── graph ──────────────────────────────────────────────────────────────

    async fn profiles() -> Vec<String> {
        services::call(
            Some(unipi_kit::suite::pkg::SUBAGENTS.id),
            "subagents.profiles",
            &json!({}),
            Some(Duration::from_secs(2)),
        )
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
    }

    async fn update_graph(&'static self, args: &Value, cx: &ToolCtx) -> ToolResult {
        let given: Vec<&str> = [
            (
                "add_work",
                args.get("add_work")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty()),
            ),
            (
                "stop",
                args.get("stop")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty()),
            ),
            ("finish", args.get("finish").is_some_and(|f| !f.is_null())),
        ]
        .into_iter()
        .filter(|g| g.1)
        .map(|g| g.0)
        .collect();
        let op = args
            .get("operation")
            .and_then(Value::as_str)
            .or_else(|| (given.len() == 1).then(|| given[0]));
        let Some(op) = op else {
            return err("Set operation (add_work, stop or finish) and the matching field.");
        };
        let available = Self::profiles().await;
        if available.is_empty() {
            return err(
                "Agent graph is unavailable: subagents are not loaded or are disabled in this session (check /settings → subagents).",
            );
        }
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(ToOwned::to_owned);
        let mut add = Vec::new();
        if op == "add_work" {
            for w in args
                .get("add_work")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let profile = s(w, "profile");
                if s(w, "operator_id").is_none()
                    && profile.as_ref().is_some_and(|p| !available.contains(p))
                {
                    return err(format!(
                        "Unknown profile \"{}\". Available: {}.",
                        profile.unwrap_or_default(),
                        available.join(", ")
                    ));
                }
                add.push(AddWork {
                    work_id: s(w, "work_id").map(|x| x.trim().to_owned()),
                    instruction: s(w, "instruction").unwrap_or_default(),
                    profile,
                    operator_id: s(w, "operator_id"),
                    input_ids: w
                        .get("input_ids")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect(),
                    selected: w
                        .get("selected_result_inputs")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|x| {
                            Some(Selected {
                                source_graph_id: s(x, "source_graph_id")?,
                                result_id: s(x, "result_id")?,
                            })
                        })
                        .collect(),
                    replaces: s(w, "replaces"),
                });
            }
        }
        let mode = match self.state.borrow().turn_mode.as_ref().map(|t| t.0) {
            Some(Mode::Swarm) => GraphMode::Swarm,
            _ => GraphMode::Graph,
        };
        let u = Update {
            mode,
            task: s(args, "task"),
            add,
            stop: if op == "stop" {
                args.get("stop")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|x| Some((s(x, "target_id")?, s(x, "reason").unwrap_or_default())))
                    .collect()
            } else {
                Vec::new()
            },
            finish: if op == "finish" {
                args.get("finish").map(|f| {
                    (
                        f.get("result_ids")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect(),
                        s(f, "reason").unwrap_or_default(),
                    )
                })
            } else {
                None
            },
        };
        let now = timers::now_ms();
        let gid = format!(
            "graph-{}-{}",
            radix36(u64::try_from(now).unwrap_or(0)),
            unipi_kit::fmt::random_hex(4)
        );
        let result = self.state.borrow_mut().graph.update(&u, &gid, now);
        let r = match result {
            Ok(r) => r,
            Err(e) => return err(format!("Graph update refused: {e}")),
        };
        for op_id in &r.stop_operators {
            let _ = services::call(
                Some(unipi_kit::suite::pkg::SUBAGENTS.id),
                point::SUBAGENTS_CANCEL,
                &json!({"id": op_id}),
                None,
            )
            .await;
        }
        if r.opened {
            let task = self
                .state
                .borrow()
                .graph
                .current
                .as_ref()
                .map(|g| g.task.clone())
                .unwrap_or_default();
            let kind = if mode == GraphMode::Swarm {
                OwnerKind::Swarm
            } else {
                OwnerKind::Graph
            };
            let o = {
                let mut st = self.state.borrow_mut();
                if st.owners.active.as_ref().is_some_and(|a| a.kind != kind) {
                    let reason = format!("paused(superseded_by:{})", kind.mode().as_str());
                    st.owners.suspend(&reason, &now_iso());
                }
                if st.owners.active.is_none() {
                    st.owners.activate(uuid(), kind, &task, &now_iso())
                } else {
                    None
                }
            };
            if o.is_some() {
                Self::owner_event("activated", o.as_ref(), None);
            }
        }
        if r.closed {
            let (task, n) = {
                let st = self.state.borrow();
                st.graph
                    .current
                    .as_ref()
                    .map(|g| {
                        (
                            g.task.clone(),
                            g.finish.as_ref().map_or(0, |f| f.result_ids.len()),
                        )
                    })
                    .unwrap_or_default()
            };
            let o = self.state.borrow_mut().owners.finish(
                "complete(finish)",
                &now_iso(),
                timers::now_ms(),
            );
            Self::owner_event("finished", o.as_ref(), Some("complete(finish)"));
            self.save();
            return text(format!(
                "Graph \"{task}\" finished with {n} selected result(s). Read anything you still need with agent_output, then deduplicate, verify, and synthesize for the user."
            ));
        }
        let started = self.reconcile(&cx.cx).await;
        let st = self.state.borrow();
        let Some(g) = st.graph.current.as_ref() else {
            return err("no graph");
        };
        let mut lines = vec![format!(
            "{} {} {} \"{}\" (revision {}): {}.",
            if r.opened { "Opened" } else { "Updated" },
            if g.mode == GraphMode::Swarm {
                "swarm"
            } else {
                "graph"
            },
            g.graph_id,
            g.task,
            g.revision,
            st.graph.counts_line(g)
        )];
        if !started.is_empty() {
            lines.push(format!("Started now: {}.", started.join(", ")));
        }
        let waiting: Vec<&str> = g
            .work
            .iter()
            .filter(|w| w.status == "queued")
            .map(|w| w.work_id.as_str())
            .collect();
        if !waiting.is_empty() {
            lines.push(format!(
                "Waiting (inputs or capacity): {}.",
                waiting.join(", ")
            ));
        }
        lines.push("The host runs the work. Call yield_agent_graph now unless you have other immediate supervision to do; you will be woken at the next checkpoint.".into());
        text(lines.join("\n"))
    }

    /// Starts every runnable work item through `subagents.graph-run`.
    async fn reconcile(&self, _cx: &Ctx) -> Vec<String> {
        let runnable = self.state.borrow().graph.runnable();
        let mut started = Vec::new();
        for (work_id, profile, resume) in runnable {
            let (title, task) = {
                let st = self.state.borrow();
                let mode = st.graph.open().map_or(GraphMode::Graph, |g| g.mode);
                (
                    format!(
                        "{} · {work_id}",
                        if mode == GraphMode::Swarm {
                            "Swarm"
                        } else {
                            "Graph"
                        }
                    ),
                    st.graph.render_task(&work_id),
                )
            };
            let mut payload = json!({"title": title, "task": task, "profile": profile});
            if let Some(r) = resume {
                payload["resume"] = json!(r);
            }
            let reply = services::call(
                Some(unipi_kit::suite::pkg::SUBAGENTS.id),
                point::SUBAGENTS_GRAPH_RUN,
                &payload,
                Some(Duration::from_secs(30)),
            )
            .await;
            match reply {
                Ok(v) if v.get("runId").and_then(Value::as_str).is_some() => {
                    self.state.borrow_mut().graph.started(
                        &work_id,
                        Ok(v["runId"].as_str().unwrap_or_default().to_owned()),
                    );
                    started.push(work_id);
                }
                Ok(v) if v.get("retryable").and_then(Value::as_bool) == Some(true) => {
                    timers::set(GRAPH_RETRY_TIMER, Duration::from_secs(5), false);
                    break;
                }
                Ok(v) => self.state.borrow_mut().graph.started(
                    &work_id,
                    Err(v
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("could not start")
                        .to_owned()),
                ),
                Err(e) => self
                    .state
                    .borrow_mut()
                    .graph
                    .started(&work_id, Err(e.to_string())),
            }
        }
        self.save();
        started
    }

    async fn run_done(&self, data: &Value, cx: &Ctx) {
        let Some(run) = data.get("runId").and_then(Value::as_str) else {
            return;
        };
        let r = data.get("result").cloned().unwrap_or_default();
        let ok = self.state.borrow_mut().graph.settle(
            run,
            r.get("status").and_then(Value::as_str).unwrap_or("failed"),
            r.get("text").and_then(Value::as_str).unwrap_or_default(),
            r.get("error").and_then(Value::as_str),
            u32::try_from(r.get("toolCalls").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0),
        );
        if !ok {
            return;
        }
        self.save();
        self.reconcile(cx).await;
        timers::set(GRAPH_WAKE_TIMER, Duration::from_millis(400), false);
    }

    async fn graph_wake(&self) {
        let cp = {
            let mut st = self.state.borrow_mut();
            let mut reported = std::mem::take(&mut st.reported_attention);
            let cp = st.graph.checkpoint(&mut reported);
            st.reported_attention = reported;
            cp
        };
        if let Some((_, t)) = cp {
            self.send(&t, "Graph checkpoint").await;
        }
    }

    // ── commands ───────────────────────────────────────────────────────────

    fn stop_owner(&self, kind: Option<OwnerKind>) -> Option<String> {
        let mut st = self.state.borrow_mut();
        let active = st.owners.active.clone()?;
        if kind.is_some_and(|k| k != active.kind) {
            return None;
        }
        let reason = "stopped(user_requested)";
        if active.kind == OwnerKind::Goal || active.kind == OwnerKind::RalphLoop {
            st.machine.clear(&now_iso());
            st.proposal = None;
        }
        if matches!(active.kind, OwnerKind::Graph | OwnerKind::Swarm) {
            st.graph.abandon(reason);
        }
        if active.kind == OwnerKind::RalphLoop
            && let Some(l) = st.ralph.as_mut()
        {
            l.status = "complete".into();
        }
        st.stash = None;
        st.session_override = None;
        st.owners.finish(reason, &now_iso(), timers::now_ms());
        drop(st);
        Self::owner_event("finished", Some(&active), Some(reason));
        if active.kind == OwnerKind::RalphLoop {
            let it = self
                .state
                .borrow()
                .ralph
                .as_ref()
                .map_or(0, |l| l.iteration);
            unipi_kit::bus::emit(&topics::RalphLoopEnd {
                name: active.label.clone(),
                reason: reason.into(),
                iterations: it,
            });
        }
        self.save();
        Some(active.owner_id)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "unipi modeHandler branches one to one"
    )]
    async fn mode_command(&self, mode: Mode, args: &str, cx: &CommandCtx) {
        self.load();
        let parts: Vec<&str> = args.split_whitespace().collect();
        let sub = parts.first().copied().unwrap_or_default();
        let def = mode;
        let notify = |t: &str| {
            let _ = ui::notify(t, "info");
        };
        if mode == Mode::Ralph && sub == "start" {
            let name = parts.get(1).copied().unwrap_or_default();
            if name.is_empty() {
                notify(&format!(
                    "Usage: /ralph start <name> <task content> — the checklist is written to {RALPH_DIR}/<name>.md."
                ));
                return;
            }
            let content = args
                .trim_start()
                .strip_prefix("start")
                .unwrap_or_default()
                .trim_start()
                .strip_prefix(name)
                .unwrap_or_default()
                .trim();
            if content.is_empty() {
                notify(&format!(
                    "Provide the task content (markdown with `- [ ]` items) after the name — it is written to {}.",
                    task_path(name)
                ));
                return;
            }
            match self.ralph_start(name, &content.replace("\\n", "\n")) {
                Ok(prompt) => {
                    self.save();
                    self.send(&prompt, "Ralph").await;
                    notify(&format!(
                        "Ralph loop \"{}\" started — iteration 1 dispatched.",
                        ralph::sanitize(name)
                    ));
                }
                Err(e) => notify(&format!("Loop not started: {e}")),
            }
            return;
        }
        if mode == Mode::Ralph && sub == "stop" {
            let parked = {
                let mut st = self.state.borrow_mut();
                if !st
                    .owners
                    .active
                    .as_ref()
                    .is_some_and(|o| o.kind == OwnerKind::RalphLoop)
                {
                    drop(st);
                    notify("No active ralph loop.");
                    return;
                }
                st.machine.pause("paused(user_requested)", &now_iso());
                st.owners.suspend("paused(user_requested)", &now_iso())
            };
            Self::owner_event("suspended", parked.as_ref(), Some("paused(user_requested)"));
            self.save();
            let it = self
                .state
                .borrow()
                .ralph
                .as_ref()
                .map_or(0, |l| l.iteration);
            notify(&if parked.is_some() {
                format!("Loop parked (iteration {it}). /ralph resume to continue.")
            } else {
                "Park slot busy — resume or clear the parked owner first.".into()
            });
            return;
        }
        if mode == Mode::Ralph && sub == "status" {
            let st = self.state.borrow();
            let msg = match &st.ralph {
                Some(l) => {
                    let (c, t, next) = ralph::progress(
                        &std::fs::read_to_string(task_guest(&l.name)).unwrap_or_default(),
                        l.items_per_iteration,
                    );
                    format!(
                        "Loop \"{}\": iteration {}, {c}/{t} items checked.\nNext: {}\n{}",
                        l.name,
                        l.iteration,
                        if next.is_empty() {
                            "(all checked)".to_owned()
                        } else {
                            next.iter().take(3).cloned().collect::<Vec<_>>().join(" · ")
                        },
                        st.owners.snapshot_text()
                    )
                }
                None => format!(
                    "No ralph loop in this session.\n{}",
                    st.owners.snapshot_text()
                ),
            };
            drop(st);
            notify(&msg);
            return;
        }
        let owner_kind = match mode {
            Mode::Goal => Some(OwnerKind::Goal),
            Mode::Swarm => Some(OwnerKind::Swarm),
            Mode::Graph => Some(OwnerKind::Graph),
            _ => None,
        };
        if mode != Mode::Ralph && sub == "stop" {
            let stopped = self.stop_owner(owner_kind);
            notify(&if stopped.is_some() {
                format!("{} stopped.", def.label())
            } else {
                format!("No active {}.", def.label().to_lowercase())
            });
            return;
        }
        if matches!(mode, Mode::Graph | Mode::Swarm) && sub == "history" && parts.len() == 1 {
            notify(&self.graph_history());
            return;
        }
        if sub == "status" || (sub.is_empty() && parts.len() <= 1) {
            let s = self.settings();
            let msg = {
                let st = self.state.borrow();
                let current = st.turn_mode.as_ref().map_or_else(
                    || "none yet".to_owned(),
                    |(m, src)| format!("{} ({src})", m.as_str()),
                );
                let graph = if matches!(mode, Mode::Graph | Mode::Swarm) {
                    st.graph
                        .status_line()
                        .map(|l| format!("\n{l}"))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                format!(
                    "Mode command: {}\nLast resolved: {current}\n{}{graph}\nJudge: {} — default mode: {}\nSettings: settings.json → extensionSettings.long-horizon",
                    def.label(),
                    st.owners.snapshot_text(),
                    if s.judge_enabled {
                        format!("on (threshold {})", s.judge_threshold)
                    } else {
                        "off".into()
                    },
                    s.default_mode.as_str()
                )
            };
            notify(&msg);
            if mode == Mode::Goal && s.goal_progress != "off" {
                self.estimate().await;
            }
            return;
        }
        if sub == "resume" {
            let resumed = {
                let mut st = self.state.borrow_mut();
                if let Some(a) = &st.owners.active {
                    let m = format!(
                        "Cannot resume: {} \"{}\" is already active.",
                        a.kind.as_str(),
                        a.label
                    );
                    drop(st);
                    notify(&m);
                    return;
                }
                let r = st.owners.resume(&now_iso());
                if r.is_some() {
                    st.machine.resume(&now_iso());
                    st.pending_explicit = r.as_ref().map(|o| o.kind.mode());
                }
                r
            };
            let Some(r) = resumed else {
                notify("No parked owner to resume.");
                return;
            };
            Self::owner_event("resumed", Some(&r), None);
            self.save();
            self.send("Continue the resumed owner from its own durable state; re-read its status before acting.", "Resume").await;
            return;
        }
        if sub == "clear" {
            let (active, cleared) = {
                let mut st = self.state.borrow_mut();
                match st.owners.active.clone() {
                    Some(a) => (Some(a), None),
                    None => {
                        let c = st.owners.clear_parked();
                        if c.as_ref().is_some_and(|c| {
                            matches!(c.kind, OwnerKind::Goal | OwnerKind::RalphLoop)
                        }) {
                            st.machine.clear(&now_iso());
                        }
                        (None, c)
                    }
                }
            };
            if let Some(a) = active {
                notify(&if a.kind == OwnerKind::RalphLoop {
                    "A ralph loop is running — /ralph stop parks it, or /regular ends it."
                        .to_owned()
                } else {
                    format!(
                        "A {0} is running — use /{0} stop to end it.",
                        a.kind.as_str()
                    )
                });
                return;
            }
            notify(&cleared.as_ref().map_or_else(
                || "Nothing to clear.".to_owned(),
                |c| format!("Cleared parked {} \"{}\".", c.kind.as_str(), c.label),
            ));
            if cleared.is_some() {
                Self::owner_event("cleared", cleared.as_ref(), None);
                self.save();
            }
            return;
        }
        let need_park = self
            .state
            .borrow()
            .owners
            .active
            .as_ref()
            .is_some_and(|a| a.kind.mode() != mode);
        if need_park {
            let reason = format!("paused(superseded_by:{})", mode.as_str());
            let parked = {
                let mut st = self.state.borrow_mut();
                let p = st.owners.suspend(&reason, &now_iso());
                if p.is_some() {
                    st.machine.pause("paused(superseded)", &now_iso());
                }
                p
            };
            match parked {
                Some(p) => Self::owner_event("suspended", Some(&p), Some(&reason)),
                None => {
                    let p = self.state.borrow().owners.parked.clone();
                    notify(&p.map_or_else(
                        || "Cannot switch: the active owner could not be parked.".to_owned(),
                        |p| {
                            format!(
                                "{} \"{}\" is parked — /{} resume or clear before parking another.",
                                p.kind.as_str(),
                                p.label,
                                p.kind.mode().as_str()
                            )
                        },
                    ));
                    return;
                }
            }
        }
        {
            let mut st = self.state.borrow_mut();
            st.pending_explicit = Some(mode);
            st.session_override = Some(mode);
            let tools = control::active_tools();
            if let Some(next) = modes::sync_mode_tools(&tools, mode) {
                let _ = control::set_active_tools(&next);
            }
        }
        self.save();
        let busy = !control::is_idle();
        let prov = Provenance {
            source: "Long-horizon".into(),
            title: Some(def.label().into()),
            synopsis: Some({
                let c = args.split_whitespace().collect::<Vec<_>>().join(" ");
                if c.chars().count() > 80 {
                    format!("{}…", c.chars().take(80).collect::<String>())
                } else {
                    c
                }
            }),
            severity: None,
        };
        let opts = oino_extension::wit::session::UserMessageOptions {
            deliver_as: busy.then_some(session::Delivery::FollowUp),
            expand_prompt_templates: false,
            provenance: Some(prov),
        };
        let _ =
            oino_extension::wit::session::send_user_message(json!(args.trim()).to_string(), opts)
                .await;
        if busy {
            notify(&format!(
                "{} queued — it starts when the current turn ends.",
                def.label()
            ));
        }
        let _ = cx;
    }

    fn graph_history(&self) -> String {
        let st = self.state.borrow();
        let all: Vec<&graph::Graph> = st.graph.current.iter().chain(&st.graph.history).collect();
        if all.is_empty() {
            return "No agent graphs in this project yet.".into();
        }
        all.iter()
            .take(10)
            .map(|g| {
                let done = g.work.iter().filter(|w| w.status == "completed").count();
                let state = g.finish.as_ref().map_or_else(
                    || {
                        g.abandoned
                            .as_ref()
                            .map_or_else(|| "open".to_owned(), |r| format!("stopped ({r})"))
                    },
                    |f| format!("finished → {}", f.result_ids.join(", ")),
                );
                format!(
                    "{}  {} {}  \"{}\"  {done}/{} done · {state}",
                    unipi_kit::fmt::iso_from_ms(g.created_at),
                    if g.mode == GraphMode::Swarm {
                        "swarm"
                    } else {
                        "graph"
                    },
                    g.graph_id,
                    g.task,
                    g.work.len()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// User-only goal progress estimate (status slot `lh-progress`).
    async fn estimate(&self) {
        let Some(g) = self.state.borrow().machine.goal.clone() else {
            return;
        };
        let _ = ui::status("lh-progress", Some("◎ estimating goal progress…"));
        let s = self.settings();
        let model = if s.progress_model.is_empty() {
            s.verifier_model
        } else {
            s.progress_model
        };
        let tail = Self::tail()
            .into_iter()
            .map(|(r, t)| format!("{r}: {}", t.chars().take(800).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Estimate how far an autonomous coding agent is toward its objective. Respond ONLY with JSON {{\"percent\": 0-100, \"summary\": \"one sentence of work done so far\"}}.\n--- OBJECTIVE ---\n{}\n--- STATE ---\nturn {}/{}, status {}\n--- RECENT CONVERSATION TAIL ---\n{tail}",
            g.objective,
            g.turn,
            g.max_turns,
            g.status.as_str()
        );
        let mut req = json!({"context": {"messages": [{"role": "user", "content": [{"type": "text", "text": prompt}], "timestamp": timers::now_ms()}]}, "options": {}});
        if !model.is_empty() {
            req["model"] = json!(model);
        }
        let parsed = models::complete(&req).await.ok().and_then(|m| {
            let t: String = m
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            let start = t.find('{')?;
            let end = t.rfind('}')?;
            serde_json::from_str::<Value>(&t[start..=end]).ok()
        });
        match parsed {
            Some(v) => {
                let pct = v
                    .get("percent")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0);
                let line = format!(
                    "◎ goal ~{pct:.0}% · {}",
                    v.get("summary").and_then(Value::as_str).unwrap_or_default()
                );
                let _ = ui::status("lh-progress", Some(&line));
                let _ = session::append_entry(
                    "unipi-progress",
                    Some(
                        &json!({"kind": "goal", "percent": pct, "summary": v.get("summary"), "objective": g.objective}),
                    ),
                );
            }
            None => {
                let _ = ui::status("lh-progress", None);
                let _ = ui::notify(
                    "Goal progress estimate failed (model unavailable or unparseable reply).",
                    "info",
                );
            }
        }
    }

    /// `/visualize-progress` (mermaid source + text fallback; no PNG in v1).
    fn visualize(&self) {
        let st = self.state.borrow();
        let mut mermaid = vec!["flowchart TD".to_owned()];
        let mut textv = Vec::new();
        if let Some(g) = &st.graph.current {
            for w in &g.work {
                mermaid.push(format!(
                    "  {}[\"{} · {}\"]",
                    w.work_id.replace('-', "_"),
                    w.work_id,
                    w.status
                ));
                for i in &w.input_ids {
                    mermaid.push(format!(
                        "  {} --> {}",
                        i.trim_start_matches("r-").replace('-', "_"),
                        w.work_id.replace('-', "_")
                    ));
                }
                textv.push(format!(
                    "- {} [{}] {}",
                    w.work_id,
                    w.status,
                    w.instruction.chars().take(80).collect::<String>()
                ));
            }
        }
        if let Some(gl) = &st.machine.goal {
            mermaid.push(format!(
                "  goal[\"goal · {} · turn {}/{}\"]",
                gl.status.as_str(),
                gl.turn,
                gl.max_turns
            ));
            textv.push(format!(
                "Goal: {} — {} (turn {}/{})",
                gl.objective,
                gl.status.as_str(),
                gl.turn,
                gl.max_turns
            ));
        }
        if let Some(l) = &st.ralph {
            let (c, t, _) = ralph::progress(
                &std::fs::read_to_string(task_guest(&l.name)).unwrap_or_default(),
                l.items_per_iteration,
            );
            mermaid.push(format!("  ralph[\"ralph {} · {c}/{t}\"]", l.name));
            textv.push(format!(
                "Ralph {}: iteration {}, {c}/{t} done",
                l.name, l.iteration
            ));
        }
        drop(st);
        if textv.is_empty() {
            let _ = ui::notify("No long-horizon progress to visualize yet.", "info");
            return;
        }
        let spec = json!({"t": "stack", "direction": "vertical", "gap": 1, "children": [
            {"t": "code", "value": mermaid.join("\n"), "language": "mermaid"},
            {"t": "markdown", "value": textv.join("\n")},
        ]});
        let _ = ui::panel_open(
            &json!({"panelId": "unipi.long-horizon/visualize", "placement": "overlay", "title": "Long-horizon progress", "spec": spec, "focus": true, "closeOnEscape": true}),
        );
    }

    async fn runner(&'static self, name: &str, args: &str) -> Option<Value> {
        self.load();
        let a: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
        let s = |k: &str| {
            a.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let out = match name {
            "goal-start" => {
                let obj = s("objective");
                if obj.is_empty() {
                    json!({"ok": false, "reason": "an objective is required"})
                } else if let Some(g) = self
                    .state
                    .borrow()
                    .machine
                    .goal
                    .clone()
                    .filter(|g| g.status == goal::Status::Paused)
                {
                    json!({"ok": false, "reason": format!("a parked goal exists (\"{}\") — resume or clear it first", g.objective)})
                } else {
                    let r = self.create_goal(&obj, None, false);
                    let g = self.state.borrow().machine.goal.clone();
                    let t = r
                        .content
                        .first()
                        .and_then(|c| c.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    if t.starts_with("Goal set") {
                        self.state.borrow_mut().pending_explicit = Some(Mode::Goal);
                        json!({"ok": true, "goalId": g.as_ref().map(|g| g.goal_id.clone()), "objective": obj})
                    } else {
                        json!({"ok": false, "reason": t})
                    }
                }
            }
            "goal-status" => match self.state.borrow().machine.goal.clone() {
                None => json!({"found": false, "reason": "no goal in this session"}),
                Some(g) if !s("goalId").is_empty() && s("goalId") != g.goal_id => {
                    json!({"found": false, "reason": format!("the active goal is {}", g.goal_id)})
                }
                Some(g) => {
                    json!({"found": true, "goalId": g.goal_id, "status": g.status.as_str(), "reason": g.reason, "objective": g.objective})
                }
            },
            "goal-pause" => {
                let mut st = self.state.borrow_mut();
                match st.machine.goal.clone() {
                    Some(g)
                        if g.status == goal::Status::Active
                            && st
                                .owners
                                .active
                                .as_ref()
                                .is_some_and(|o| o.kind == OwnerKind::Goal) =>
                    {
                        st.machine.pause("paused(user_requested)", &now_iso());
                        st.owners.suspend("paused(user_requested)", &now_iso());
                        json!({"ok": true, "goalId": g.goal_id, "status": "paused"})
                    }
                    Some(g) => {
                        json!({"ok": false, "reason": format!("goal is {}", g.status.as_str())})
                    }
                    None => json!({"ok": false, "reason": "no goal in this session"}),
                }
            }
            "goal-resume" => {
                let mut st = self.state.borrow_mut();
                match st.machine.goal.clone() {
                    None => json!({"ok": false, "reason": "no goal in this session"}),
                    Some(g) if g.status != goal::Status::Paused => {
                        json!({"ok": false, "reason": format!("goal is {}, not paused", g.status.as_str())})
                    }
                    Some(g) => {
                        st.machine.resume(&now_iso());
                        if st.owners.parked.is_some() {
                            st.owners.resume(&now_iso());
                        }
                        st.pending_explicit = Some(Mode::Goal);
                        json!({"ok": true, "goalId": g.goal_id, "status": "active"})
                    }
                }
            }
            "goal-stop" => {
                let gid = self
                    .state
                    .borrow()
                    .machine
                    .goal
                    .as_ref()
                    .map(|g| g.goal_id.clone());
                match self.stop_owner(Some(OwnerKind::Goal)) {
                    Some(_) => json!({"ok": true, "goalId": gid}),
                    None => json!({"ok": false, "reason": "no active goal"}),
                }
            }
            "lh-explicit" | "lh-mode" => {
                let m = s("mode");
                match Mode::parse(&m) {
                    None => json!({"ok": false, "reason": format!("unknown mode \"{m}\"")}),
                    Some(mode) => {
                        let owner = self.state.borrow().owners.active.as_ref().map(|o| o.kind);
                        if mode != Mode::None
                            && owner.is_some_and(|k| k.mode() != mode)
                            && name == "lh-explicit"
                        {
                            json!({"ok": false, "reason": format!("the session is owned by a {} owner — finish or suspend it first", owner.map_or("", |k| k.as_str()))})
                        } else {
                            let mut st = self.state.borrow_mut();
                            st.pending_explicit = Some(mode);
                            st.session_override = Some(mode);
                            json!({"ok": true, "mode": m})
                        }
                    }
                }
            }
            "ralph-start" => {
                let (n, c) = (
                    s("name"),
                    a.get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                );
                if n.is_empty() || c.trim().is_empty() {
                    json!({"ok": false, "reason": "name and content are required"})
                } else {
                    match self.ralph_start(&n, &c) {
                        Ok(p) => {
                            self.save();
                            self.send(&p, "Ralph").await;
                            json!({"ok": true})
                        }
                        Err(e) => json!({"ok": false, "reason": e}),
                    }
                }
            }
            "lh-graph" => match s("op").as_str() {
                "output" => match self
                    .state
                    .borrow()
                    .graph
                    .read_result(&s("id"), a.get("graphId").and_then(Value::as_str))
                {
                    None => json!({"ok": false, "reason": "That step is gone."}),
                    Some((gid, w)) => {
                        json!({"ok": true, "output": {"graphId": gid, "id": w.work_id, "status": w.status, "resultId": w.result_id, "operatorId": w.operator_id, "instruction": w.instruction, "text": w.result, "failure": w.failure, "truncated": w.result_truncated}})
                    }
                },
                "stop" => {
                    let open = self.state.borrow().graph.open().map(|g| g.mode);
                    match open {
                        None => json!({"ok": false, "reason": "No agent graph is running."}),
                        Some(m) => {
                            if self
                                .stop_owner(Some(if m == GraphMode::Swarm {
                                    OwnerKind::Swarm
                                } else {
                                    OwnerKind::Graph
                                }))
                                .is_none()
                            {
                                self.state
                                    .borrow_mut()
                                    .graph
                                    .abandon("stopped(user_requested)");
                                self.save();
                            }
                            json!({"ok": true})
                        }
                    }
                }
                "history" => json!({"ok": true, "runs": self.state.borrow().graph.history_json()}),
                op => json!({"ok": false, "reason": format!("unknown op \"{op}\"")}),
            },
            _ => return None,
        };
        self.save();
        Some(out)
    }

    async fn session_start(&self, ev: &Event) {
        {
            let mut st = self.state.borrow_mut();
            st.loaded = false;
            if matches!(
                ev.get("reason").and_then(Value::as_str),
                Some("new" | "resume" | "fork")
            ) && st.session_override.is_some_and(|m| m != Mode::None)
            {
                st.session_override = None;
                st.pending_explicit = None;
            }
        }
        self.load();
        let tools = control::active_tools();
        let owner_mode = self
            .state
            .borrow()
            .owners
            .active
            .as_ref()
            .map_or(Mode::None, |o| o.kind.mode());
        if let Some(next) = modes::sync_mode_tools(&tools, owner_mode) {
            let _ = control::set_active_tools(&next);
        }
        // Crash recovery: re-deliver the durable outbox (at most twice), else arm a recovery fragment.
        let redeliver = {
            let mut st = self.state.borrow_mut();
            st.display = Some(st.session_override.unwrap_or(self.settings().default_mode));
            let driver = st.owners.active.as_ref().map(|o| o.kind);
            match st.machine.active().cloned() {
                Some(g) if matches!(g.status, goal::Status::Active | goal::Status::Waiting) => {
                    match g.pending_continuation.clone() {
                        Some(_)
                            if !matches!(driver, Some(OwnerKind::Goal | OwnerKind::RalphLoop)) =>
                        {
                            st.machine.clear_pending();
                            st.recovery_armed = true;
                            None
                        }
                        None => {
                            st.recovery_armed = true;
                            None
                        }
                        Some(p) if p.attempts >= goal::MAX_RECOVERY_REDELIVERIES => {
                            st.machine.pause("paused(recovery_exhausted)", &now_iso());
                            let now = now_iso();
                            if st
                                .owners
                                .suspend("paused(recovery_exhausted)", &now)
                                .is_none()
                            {
                                st.owners.finish(
                                    "paused(recovery_exhausted)",
                                    &now,
                                    timers::now_ms(),
                                );
                            }
                            st.recovery_armed = false;
                            let _ = ui::notify(
                                &format!(
                                    "Goal continuation could not be re-delivered after {} restarts — goal paused. /goal resume to continue.",
                                    p.attempts
                                ),
                                "warning",
                            );
                            None
                        }
                        Some(p) => {
                            if g.status == goal::Status::Waiting {
                                st.machine.wake(&now_iso());
                            }
                            let t = if p.text.starts_with(goal::RECOVERY_FRAGMENT) {
                                p.text
                            } else {
                                format!("{}\n\n{}", goal::RECOVERY_FRAGMENT, p.text)
                            };
                            st.machine.record_pending(&t, p.attempts + 1);
                            st.recovery_armed = false;
                            Some(t)
                        }
                    }
                }
                _ => None,
            }
        };
        // Interrupted graph steps resume once.
        {
            let mut st = self.state.borrow_mut();
            if let Some(g) = st
                .graph
                .current
                .as_mut()
                .filter(|g| g.finish.is_none() && g.abandoned.is_none())
            {
                for w in g.work.iter_mut().filter(|w| w.status == "running") {
                    if w.restart_resumes < 1 {
                        w.status = "queued".into();
                        w.resume_operator_id = w.operator_id.clone();
                        w.restart_resumes += 1;
                    } else {
                        w.status = "failed".into();
                        w.failure = Some("interrupted by a restart".into());
                    }
                }
            }
        }
        self.save();
        if let Some(t) = redeliver {
            self.send(&t, "Recovery").await;
        }
    }
}

fn radix36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(D[usize::try_from(n % 36).unwrap_or(0)]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Hidden runner commands (unipi `registerCommandRunner`).
pub const RUNNERS: &[&str] = &[
    "goal-start",
    "goal-status",
    "goal-pause",
    "goal-resume",
    "goal-stop",
    "lh-explicit",
    "lh-mode",
    "ralph-start",
    "lh-graph",
];

impl Extension for Lh {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = modes::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        for (name, label, description, params) in tools::all() {
            cx.tool(
                Tool::<Self>::new(name)
                    .label(label)
                    .description(description)
                    .parameters(&params)
                    .inactive()
                    .run(move |ext, args: Value, cx| async move {
                        ext.run_tool(name, &args, &cx).await
                    }),
            );
        }
        for mode in [Mode::Goal, Mode::Ralph, Mode::Swarm, Mode::Graph] {
            cx.command(
                Command::<Self>::new(mode.as_str())
                    .description(tools::mode_description(mode.as_str()))
                    .argument_hint(if mode == Mode::Ralph {
                        "start <name> | stop | status | resume | clear | <prompt>"
                    } else {
                        "<prompt> | status | stop | resume | clear"
                    })
                    .run(move |ext, args, cx| async move {
                        ext.mode_command(mode, &args, &cx).await;
                        Ok(())
                    })
                    .complete(move |_e, prefix, _cx| async move {
                        tools::completions(mode.as_str(), &prefix)
                            .into_iter()
                            .map(|(v, l, d)| CompletionItem {
                                value: v,
                                label: Some(l),
                                description: Some(d),
                            })
                            .collect()
                    }),
            );
        }
        cx.command(Command::<Self>::new("regular").description("Regular mode — stop any active owner and run prompts without long-horizon routing")
            .run(|ext, _a, _cx| async move {
                ext.load();
                ext.stop_owner(None);
                {
                    let mut st = ext.state.borrow_mut();
                    st.pending_explicit = Some(Mode::None);
                    st.session_override = Some(Mode::None);
                }
                if let Some(next) = modes::sync_mode_tools(&control::active_tools(), Mode::None) {
                    let _ = control::set_active_tools(&next);
                }
                ext.save();
                ui::notify("Regular mode.", "info")
            }));
        cx.command(
            Command::<Self>::new("visualize-progress")
                .description("Show long-horizon progress (goal, ralph checklist, agent graph)")
                .run(|ext, _a, _cx| async move {
                    ext.load();
                    ext.visualize();
                    Ok(())
                }),
        );
        for r in RUNNERS {
            cx.command(
                Command::<Self>::new(*r)
                    .description("long-horizon runner")
                    .hidden()
                    .run_value(move |ext, args, _cx| async move { Ok(ext.runner(r, &args).await) }),
            );
        }
        cx.nudge_provider(|ext, cx| async move {
            if cx.is_child() {
                return None;
            }
            let stash = ext.state.borrow().stash.clone()?;
            Some(json!({"id": goal::digest_hex(stash.text.as_bytes()), "priority": 100, "customType": "unipi:lh-continue", "content": stash.text, "display": true,
                "details": {"oinoProvenance": {"extensionId": EXTENSION_ID, "source": "Long-horizon", "title": stash.title, "delivery": "boundary"}}}))
        });
        cx.on_nudge_delivered(|ext, _id, _cx| {
            let mut st = ext.state.borrow_mut();
            st.stash = None;
            st.machine.clear_pending();
        });
        cx.service(fan::COMPACTION_CONTEXT_PROVIDE, |ext, _p, _c, _cx| async move {
            ext.load();
            let st = ext.state.borrow();
            let clip = |t: &str, n: usize| { let f = t.split_whitespace().collect::<Vec<_>>().join(" "); if f.chars().count() <= n { f } else { format!("{}…", f.chars().take(n - 1).collect::<String>()) } };
            let text = match (&st.ralph, st.machine.active()) {
                (Some(l), _) if l.status == "active" => Some(format!("Ralph loop \"{}\" is running — iteration {}{}.\nTask file (the checklist is the source of truth, re-read it): {RALPH_DIR}/{}\nWork the next unchecked items, update the task file, then call ralph_done.", l.name, l.iteration, if l.max_iterations > 0 { format!("/{}", l.max_iterations) } else { String::new() }, l.task_file)),
                (_, Some(g)) if g.status == goal::Status::Active => Some(format!("Goal (active, turn {}/{}{}): \"{}\"\nKeep working toward it; call get_goal for the durable state. Completion is judged by an independent verifier.", g.turn, g.max_turns, if g.no_progress_streak > 0 { format!(", {}/{} no-progress", g.no_progress_streak, g.stall_cap) } else { String::new() }, clip(&g.objective, 700))),
                _ => None,
            };
            Ok(serde_json::to_value(CompactionContext { id: "long-horizon".into(), text })?)
        });
        cx.on(EventKind::BeforeAgentStart, |ext, ev, cx| async move {
            Ok(ext.gate(&ev, &cx).await)
        });
        cx.on(EventKind::ToolCall, |ext, ev, _cx| async move {
            Ok(ext.tool_call(&ev))
        });
        cx.on(EventKind::ToolResult, |ext, ev, _cx| async move {
            ext.tool_result(&ev).await;
            Ok(())
        });
        cx.on(EventKind::AgentBeforeSettle, |ext, ev, cx| async move {
            if cx.is_child() {
                return Ok(());
            }
            let aborted = ev
                .get("outcome")
                .and_then(Value::as_str)
                .is_some_and(|o| o != "completed");
            ext.goal_turn_end(aborted).await;
            if ext.settings().goal_progress == "loop"
                && ext.state.borrow().machine.active().is_some()
                && ext
                    .state
                    .borrow()
                    .owners
                    .active
                    .as_ref()
                    .is_some_and(|o| o.kind == OwnerKind::Goal)
            {
                spawn(async move { ext.estimate().await });
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            match ev.get("timerId").and_then(Value::as_str) {
                Some(WAIT_TIMER) => {
                    let woke = ext.state.borrow_mut().machine.wake(&now_iso());
                    if let Some(g) = woke {
                        ext.state.borrow_mut().waits = 0;
                        let hint = goal::continuation_hint(&g, None);
                        ext.state.borrow_mut().machine.record_pending(&hint, 0);
                        ext.save();
                        Lh::send_now(&hint, "Continuation").await;
                    }
                }
                Some(GRAPH_WAKE_TIMER) => ext.graph_wake().await,
                Some(GRAPH_RETRY_TIMER) => {
                    ext.reconcile(&cx).await;
                }
                _ => {}
            }
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            if ev.get("topic").and_then(Value::as_str) == Some(topics::SUBAGENTS_RUN_DONE) {
                ext.run_done(ev.get("data").unwrap_or(&Value::Null), &cx)
                    .await;
            }
            Ok(())
        });
        bus::subscribe(topics::SUBAGENTS_RUN_DONE);
        cx.on(EventKind::SessionStart, |ext, ev, _cx| async move {
            ext.session_start(&ev).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, _cx| async move {
            ext.save();
            Ok(())
        });
        cx.on(EventKind::SessionCompact, |ext, _ev, _cx| async move {
            ext.state.borrow_mut().recovery_armed = true;
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Lh);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_eq!(task_path("My Loop"), ".oino/ralph/my-loop.md");
        assert_eq!(RUNNERS.len(), 9);
    }
}
