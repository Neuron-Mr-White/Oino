//! `unipi.long-horizon` — goal / ralph / swarm / graph owners (unipi `packages/long-horizon`,
//! unipi-port §3.14).
//!
//! | unipi | here |
//! |---|---|
//! | gate at `before_agent_start` (child → explicit → owner → jev judge → default), mode section, tool exposure | [`Lh::gate`]; `update-active-tools` naming only mode tools (no payload surgery, unipi-port §2.5); `tool_call` blocks hidden mode tools. Mode state (pending/override/turn mode) is per session id (F-06): children share the instance and never read or clobber the lead's mode |
//! | owner coordinator (`state.json`, leases, history 10) | [`modes::Owners`] in `/ext-workspace/<wsid>/state.json` |
//! | goal machine, verifier, continuation, task gate, wait backoff, recovery outbox (`goal.json`) | [`goal`]; verifier through `models.complete` |
//! | ralph task files `.unipi/ralph/<name>.md` | `.oino/ralph/<name>.md` (+ `.state.json`) ([`ralph`]) |
//! | agent graph (`graph.json`, history 20) over the subagents graph-run port | [`graph`]; service `subagents.graph-run`, bus `unipi:subagents:run-done` |
//! | transient-failure recovery (D6): classify, 3 retries from the node's own session (backoff 30 s/2 m/10 m), Retry-After honored without budget (1 h cap), then failed + blocked downstream + nudge + notify | [`graph`] ([`graph::classify_failure`], retry fields in [`graph::Work`]); timer wiring in `run_done`/`reconcile`; sticky `unipi:long-horizon:work-failed` |
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
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::session::{CustomMessage, Delivery, Provenance, SendOptions};
use oino_extension::ui::widgets::prelude::*;
use unipi_kit::services::{CompactionContext, fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::topics;
use unipi_kit::{fanout, jev};

use crate::goal::{Evidence, Hint, Machine, Settlement};
use crate::graph::{
    AddWork, By, GraphFile, GraphMode, LhCode, LhError, Selected, Update, finish_args,
};
use crate::modes::{EXTENSION_ID, Mode, NAMESPACE, OwnerKind, Owners, Settings};
use crate::ralph::{LoopState, RALPH_DIR, RALPH_GUEST_DIR};

const WAIT_TIMER: &str = "lh-goal-wait";
const GRAPH_WAKE_TIMER: &str = "lh-graph-wake";
const GRAPH_RETRY_TIMER: &str = "lh-graph-retry";
/// D6 §9.2: per-work transient-retry timers are `<prefix><work id>`.
const RETRY_TIMER_PREFIX: &str = "lh-retry-";
/// D6 §9.3: sticky terminal-failure topic (published directly — not a unipi-kit topic).
const WORK_FAILED_TOPIC: &str = "unipi:long-horizon:work-failed";
/// Sticky topic: a graph work item stopped on an error (`errored`, continuable).
const WORK_ERRORED_TOPIC: &str = "unipi:long-horizon:work-errored";
/// The app's graph data + Retry service (`_oino/service_call`).
pub const GRAPH_SERVICE: &str = "long-horizon.graph";
/// Auto-continue timer of a goal / ralph turn that ended on an error.
const GOAL_CONTINUE_TIMER: &str = "lh-goal-continue";
/// Graph events in the `/visualize-progress` params.
const VISUAL_GRAPH_EVENTS: usize = 16;
/// Early run-done payloads kept for a start that has not been recorded yet.
const EARLY_DONE_MAX: usize = 8;
/// Errored work ids listed in the `LH_PROGRESS` graph snapshot.
const PROGRESS_ERRORED_MAX: usize = 5;
const VERIFIER_TIMEOUT_MS: u64 = 30_000;
const REMIND_AFTER: usize = 3;
/// Second attempt for the `subagents.profiles` lookup after the 2 s probe misses on a
/// cold start (services not registered yet).
const PROFILE_LOOKUP_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
/// Pause before the profile lookup retry (a waking subagents instance replays its
/// session start first).
const PROFILE_LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Subagents run-done outcomes that mean the worker crashed / the process went away
/// without a settle (D6 §9.1 row 1: transient, no retry-after).
const WORKER_CRASHED_NEEDLES: &[&str] = &["session ended", "interrupted"];

fn worker_crashed(error: Option<&str>) -> bool {
    error.is_some_and(|e| {
        let lower = e.to_lowercase();
        WORKER_CRASHED_NEEDLES.iter().any(|n| lower.contains(n))
    })
}

/// The operator session cannot be resumed at all (D6 §9.2): its record is gone
/// ("No subagent found … — can't resume.") or its session file is unreadable —
/// a retry may spawn a fresh operator, once.
fn resume_unrecoverable(error: &str) -> bool {
    const NEEDLES: &[&str] = &["can't resume", "session file", "corrupt", "unreadable"];
    let lower = error.to_lowercase();
    NEEDLES.iter().any(|n| lower.contains(n))
}

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
    /// Mode state is per session (F-06): child workers share this one extension
    /// instance, so a child's gate must never read, clear or clobber the lead's
    /// pending/override/turn mode. Keyed by session id.
    pending_explicit: std::collections::HashMap<String, Mode>,
    session_override: std::collections::HashMap<String, Mode>,
    display: Option<Mode>,
    last_badge: Option<Mode>,
    turn_modes: std::collections::HashMap<String, (Mode, String)>,
    turns: std::collections::HashMap<String, Turn>,
    proposal: Option<(String, Option<String>, bool)>,
    waits: u32,
    verifier_reason: Option<String>,
    recovery_armed: bool,
    reminders: std::collections::HashMap<String, u32>,
    reported_attention: Vec<String>,
    judge_cache: Option<(String, Mode, f64, i64)>,
    loaded: bool,
    /// Session that owns the live graph (the lead). Event deliveries and retry timers
    /// can arrive with a stale nested-session context (E-02), so every graph-run start
    /// names this session explicitly in the `subagents.graph-run` payload. In-memory
    /// only: a fresh bind learns it from the parent's `session_start`.
    graph_session: Option<String>,
    /// Run-done payloads that arrived before their start was recorded (a worker can
    /// finish while `subagents.graph-run` is still answering): replayed once the start
    /// lands, at most [`EARLY_DONE_MAX`] (oldest dropped).
    early_done: Vec<Value>,
}

/// The extension.
#[derive(Default)]
pub struct Lh {
    settings: Store,
    state: RefCell<State>,
}

/// Puts exactly `mode`'s control tools among the mode tools, through the host's atomic
/// `update-active-tools`: only mode tools are named, so a stale or empty active-tools read
/// can never take the core tools (read/bash/edit/write) or other extensions' tools away.
fn apply_mode_tools(mode: Mode) {
    if let Some((enable, disable)) = modes::mode_tool_update(&control::active_tools(), mode) {
        let _ = control::update_active_tools(&enable, &disable);
    }
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

/// A mode-command status as a transcript row (unipi prints these in the chat; a long
/// `notify` would open the blocking result-card modal and swallow the next typed
/// command — D-2). Same shape as the utility package's `respond`.
async fn respond(markdown: &str) {
    let msg = CustomMessage {
        custom_type: "unipi-response".into(),
        content: json!(markdown),
        display: true,
        details: Some(json!({
            "unipiHarness": {
                "version": 1,
                "id": format!("hm-{}", unipi_kit::fmt::random_hex(8)),
                "source": "Long-horizon",
                "title": "Response",
                "synopsis": "Mode command response",
                "delivery": "followUp"
            }
        })),
    };
    let options = SendOptions {
        trigger_turn: false,
        deliver_as: Some(Delivery::FollowUp),
        provenance: Some(Provenance {
            source: "Long-horizon".into(),
            title: Some("Response".into()),
            synopsis: Some("Mode command response".into()),
            severity: None,
        }),
    };
    let _ = session::send_message(&msg, options).await;
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
        st.graph.policy = graph::Policy {
            auto_continue: self.settings().auto_continue,
        };
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
                "goal": st.machine.goal.as_ref().map(|g| {
                    let mut v = json!({"objective": g.objective, "status": if g.errored.is_some() && !g.status.terminal() { "errored" } else { g.status.as_str() }, "turn": g.turn, "maxTurns": g.max_turns});
                    if let Some(e) = &g.errored {
                        v["error"] = json!(e.chars().take(200).collect::<String>());
                    }
                    v
                }),
                "ralph": ralph_progress,
                "graph": st.graph.current.as_ref().map(|g| {
                    let mut v = json!({"graphId": g.graph_id, "mode": g.mode, "task": g.task, "counts": st.graph.counts(g)});
                    let errored: Vec<Value> = g.work.iter().filter(|w| w.status == "errored").take(PROGRESS_ERRORED_MAX).map(|w| json!({"workId": w.work_id, "error": w.failure.as_deref().map(|f| f.chars().take(200).collect::<String>())})).collect();
                    if !errored.is_empty() {
                        v["errored"] = json!(errored);
                    }
                    v
                }),
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
        let session = cx.session_id().to_owned();
        let (mode, source, confidence) = if cx.is_child() {
            (Mode::None, "child", None)
        } else {
            let (fresh, sticky) = {
                let st = self.state.borrow();
                (
                    st.pending_explicit.get(&session).copied(),
                    st.session_override.get(&session).copied(),
                )
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
            self.state.borrow_mut().pending_explicit.remove(&session);
            let sticky_badge = fresh.is_none() && sticky.is_some() && explicit == sticky;
            if sticky_badge { (r.0, r.1, r.2) } else { r }
        };
        apply_mode_tools(mode);
        let show_badge = {
            let mut st = self.state.borrow_mut();
            st.display = Some(mode);
            st.turn_modes
                .insert(session.clone(), (mode, source.to_owned()));
            let show = source == "explicit" || st.last_badge != Some(mode);
            st.last_badge = Some(mode);
            st.turns.insert(session, Turn::default());
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

    fn tool_call(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let name = ev
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = ev.get("input").cloned().unwrap_or(Value::Null);
        let mut st = self.state.borrow_mut();
        let (mode, _) = st.turn_modes.get(cx.session_id()).cloned()?;
        if modes::hidden_tools(mode).contains(&name) {
            return Some(
                json!({"block": true, "reason": format!("long-horizon: \"{name}\" is not available in {} mode. Switch with /goal, /ralph, /swarm, or /graph.", mode.as_str())}),
            );
        }
        if mode == Mode::None {
            return None;
        }
        let turn = st.turns.entry(cx.session_id().to_owned()).or_default();
        turn.tool_calls += 1;
        if let Some(c) = input.get("command").and_then(Value::as_str) {
            turn.commands.push(c.chars().take(200).collect());
        }
        if matches!(name, "edit" | "write")
            && let Some(p) = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .and_then(Value::as_str)
        {
            turn.files.push(p.chars().take(200).collect());
        }
        None
    }

    async fn tool_result(&self, ev: &Event, cx: &Ctx) {
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
            if st.owners.active.is_none() {
                return;
            }
            let Some(turn) = st.turns.get_mut(cx.session_id()) else {
                return;
            };
            if turn.steered {
                return;
            }
            turn.steps.push(step);
            match ralph::detect(&turn.steps, REMIND_AFTER) {
                Some((sig, n)) => {
                    turn.steered = true;
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
        // perf-fix #1: the newest user/assistant messages only (5 with text are kept).
        session::query(&session::EntryQuery::messages(&["user", "assistant"]).last(32))
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
    async fn goal_turn_end(&self, aborted: bool, session: &str) {
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
            let activity = st
                .turns
                .get(session)
                .map(|t| (t.tool_calls, t.files.clone(), t.commands.clone()))
                .unwrap_or_default();
            (st.proposal.take(), activity)
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
            "ralph_done" => self.ralph_done(cx.session_id()).await,
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
                        let errored = st.graph.errored();
                        if !st.graph.can_progress() && !errored.is_empty() {
                            // Errored work never moves by itself: yielding would stall.
                            err(format!(
                                "Nothing can progress: {} errored (not failed; sessions kept). Continue it with update_agent_graph operation=continue continue=[{{target_id}}], fail it with operation=fail if you give up, or stop it.",
                                errored
                                    .iter()
                                    .map(|(id, e)| format!(
                                        "{id} ({})",
                                        e.chars().take(160).collect::<String>()
                                    ))
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            ))
                        } else if running == 0 && queued == 0 {
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
                            let partial = if w.status == "errored" {
                                format!(
                                    "\nError (continuable): {}\nPartial output so far:",
                                    w.failure.clone().unwrap_or_else(|| "unknown".into())
                                )
                            } else {
                                String::new()
                            };
                            text(format!(
                                "{head}{}{partial}\n\n{body}",
                                if w.result_truncated {
                                    " · stored text truncated"
                                } else {
                                    ""
                                }
                            ))
                        } else if matches!(w.status.as_str(), "failed" | "errored") {
                            text(format!(
                                "{head}\n{}: {}",
                                if w.status == "errored" {
                                    "Error (continuable)"
                                } else {
                                    "Failure"
                                },
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

    fn ralph_start(
        &self,
        name: &str,
        content: &str,
        session: &str,
    ) -> std::result::Result<String, String> {
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
        st.session_override.insert(session.to_owned(), Mode::Ralph);
        drop(st);
        Self::owner_event("activated", o.as_ref(), None);
        unipi_kit::bus::emit_raw(
            topics::RALPH_ITERATION_DONE,
            &json!({"name": state.name, "iteration": 0, "remaining": items.len()}),
        );
        Ok(prompt)
    }

    async fn ralph_done(&self, session: &str) -> ToolResult {
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
                    st.session_override.remove(session);
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

    /// The subagents profile list. On a cold host the subagents extension may not have
    /// registered its services yet (startup race), and a subagents instance that just woke
    /// from hibernation is still replaying its session start: the first short probe then
    /// fails or comes back empty, so after a short pause one retry with a longer timeout
    /// runs before giving up. The error carries the last failure text ("timed out" ⇒
    /// subagents are still starting).
    async fn profiles() -> Result<Vec<String>, String> {
        let mut last = String::from("no profiles service call was made");
        for (attempt, timeout) in [Duration::from_secs(2), PROFILE_LOOKUP_RETRY_TIMEOUT]
            .into_iter()
            .enumerate()
        {
            if attempt > 0 {
                timers::sleep(PROFILE_LOOKUP_RETRY_DELAY).await;
            }
            match services::call(
                Some(unipi_kit::suite::pkg::SUBAGENTS.id),
                "subagents.profiles",
                &json!({}),
                Some(timeout),
            )
            .await
            {
                Ok(v) => match serde_json::from_value::<Vec<String>>(v) {
                    Ok(p) if !p.is_empty() => return Ok(p),
                    Ok(_) => {
                        last = "subagents answered with no profiles for this session \
                                (subagents are disabled in /settings → subagents)"
                            .into();
                    }
                    Err(e) => last = format!("subagents.profiles answered garbage: {e}"),
                },
                Err(e) => last = format!("subagents.profiles failed: {e}"),
            }
        }
        Err(last)
    }

    /// Ids of the subagents runs still running in this session (empty when subagents
    /// do not answer): the graph snapshot surfaces work whose worker outlived its item.
    async fn live_operators() -> HashSet<String> {
        let Ok(records) = services::call(
            Some(unipi_kit::suite::pkg::SUBAGENTS.id),
            point::SUBAGENTS_LIST,
            &json!({}),
            Some(Duration::from_secs(2)),
        )
        .await
        else {
            return HashSet::new();
        };
        records
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r.get("status").and_then(Value::as_str) == Some("running"))
            .filter_map(|r| r.get("id").and_then(Value::as_str).map(ToOwned::to_owned))
            .collect()
    }

    async fn update_graph(&'static self, args: &Value, cx: &ToolCtx) -> ToolResult {
        // The calling session owns whatever graph changes follow (tools are not
        // registered for child sessions; the check keeps a stray delivery honest).
        if !cx.cx.is_child() {
            self.state.borrow_mut().graph_session = Some(cx.cx.session_id().to_owned());
        }
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
            (
                "continue",
                args.get("continue")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty()),
            ),
            (
                "fail",
                args.get("fail")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty()),
            ),
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
            return err(
                "Set operation (add_work, stop, continue, fail or finish) and the matching field.",
            );
        };
        // The profile list is only needed to schedule or run work: stop / fail / finish
        // also work while subagents are down.
        let available = match if matches!(op, "add_work" | "continue") {
            Self::profiles().await
        } else {
            Ok(Vec::new())
        } {
            Ok(p) => p,
            // The retry also timed out: subagents exist but are still starting up —
            // this is transient, not a config problem.
            Err(e) if e.contains("timed out") => {
                return err(
                    "Agent graph is unavailable: subagents are still starting — try again.",
                );
            }
            Err(e) => {
                return err(format!(
                    "Agent graph is unavailable: {e}. Check /settings → subagents, then try again."
                ));
            }
        };
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
        let mode = match self
            .state
            .borrow()
            .turn_modes
            .get(cx.session_id())
            .map(|t| t.0)
        {
            Some(Mode::Swarm) => GraphMode::Swarm,
            _ => GraphMode::Graph,
        };
        let targets = |key: &str, note: &str| -> Vec<(String, Option<String>, bool)> {
            args.get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    Some((
                        s(x, "target_id")?.trim().to_owned(),
                        s(x, note),
                        x.get("fail").and_then(Value::as_bool) == Some(true),
                    ))
                })
                .collect()
        };
        // `stop` with `fail:true` is the explicit give-up too (same as operation=fail).
        let (stop_fail, stop): (Vec<_>, Vec<_>) = if op == "stop" {
            targets("stop", "reason").into_iter().partition(|t| t.2)
        } else {
            (Vec::new(), Vec::new())
        };
        let mut fail: Vec<(String, String)> = stop_fail
            .into_iter()
            .map(|(t, r, _)| (t, r.unwrap_or_default()))
            .collect();
        if op == "fail" {
            fail.extend(
                targets("fail", "reason")
                    .into_iter()
                    .map(|(t, r, _)| (t, r.unwrap_or_default())),
            );
        }
        let u = Update {
            mode,
            task: s(args, "task"),
            add,
            stop: stop
                .into_iter()
                .map(|(t, r, _)| (t, r.unwrap_or_default()))
                .collect(),
            resume: if op == "continue" {
                targets("continue", "note")
                    .into_iter()
                    .map(|(t, n, _)| (t, n))
                    .collect()
            } else {
                Vec::new()
            },
            fail,
            finish: if op == "finish" {
                // G-11: the schema's nested `finish:{result_ids, reason}` and the flat
                // top-level `{operation:"finish", result_ids, reason}` both parse — the
                // flat form used to be refused ("nothing to do") and cost a failed turn.
                finish_args(args)
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
        if r.opened && !cx.cx.is_child() {
            self.state.borrow_mut().graph.set_owner(cx.cx.session_id());
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
        // The graph is persisted before any worker spawns: a spawned worker's child
        // session start re-reads the workspace state in this same process, and an
        // unsaved graph would then vanish from under the reconcile below (the model
        // would see "no graph" right after opening one).
        self.save();
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
        if !r.continued.is_empty() {
            lines.push(format!(
                "Continued on the same operator session: {}.",
                r.continued.join(", ")
            ));
        }
        if !started.is_empty() {
            lines.push(format!("Started now: {}.", started.join(", ")));
        }
        let errored = st.graph.errored();
        if !errored.is_empty() {
            lines.push(format!(
                "Errored (continue or fail them): {}.",
                errored
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
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

    /// Starts every runnable work item through `subagents.graph-run` (work waiting on a
    /// scheduled continue runs only once its fire time is due). A transport error (the
    /// subagents service not bound yet, right after a restart) keeps the work queued and
    /// re-arms the reconcile timer, bounded per item; a refused start is `errored`.
    async fn reconcile(&self, _cx: &Ctx) -> Vec<String> {
        let runnable = self.state.borrow().graph.runnable(timers::now_ms());
        let mut started = Vec::new();
        for (work_id, profile, resume) in runnable {
            let (title, task) = self.run_payload(&work_id);
            match self
                .graph_run(&title, &task, &profile, resume.as_deref())
                .await
            {
                Ok(v) if v.get("runId").and_then(Value::as_str).is_some() => {
                    let mut st = self.state.borrow_mut();
                    st.graph.started(
                        &work_id,
                        Ok(v["runId"].as_str().unwrap_or_default().to_owned()),
                        timers::now_ms(),
                    );
                    st.graph.set_ran(&work_id, graph::RanOn::from_reply(&v));
                    drop(st);
                    started.push(work_id);
                }
                Ok(v) if v.get("retryable").and_then(Value::as_bool) == Some(true) => {
                    timers::set(GRAPH_RETRY_TIMER, Duration::from_secs(5), false);
                    break;
                }
                Ok(v) => {
                    let error = v
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("could not start")
                        .to_owned();
                    if let Some(id) = self
                        .start_after_unrecoverable(&work_id, &profile, &error)
                        .await
                    {
                        started.push(id);
                    }
                }
                Err(e) => {
                    let e = e.to_string();
                    let queued =
                        self.state
                            .borrow_mut()
                            .graph
                            .start_error(&work_id, &e, timers::now_ms());
                    if queued {
                        timers::set(GRAPH_RETRY_TIMER, Duration::from_secs(5), false);
                        break;
                    }
                    self.save();
                    self.notify_work(&work_id);
                }
            }
        }
        self.save();
        self.replay_early_done();
        started
    }

    /// `(title, task)` for a graph-run of `work_id`.
    fn run_payload(&self, work_id: &str) -> (String, String) {
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
            st.graph.render_task(work_id),
        )
    }

    /// One `subagents.graph-run` call. The owner session (the lead) is declared in the
    /// payload so the run records under the lead even when the delivery context is a
    /// stale nested session (E-02); `None` lets subagents use the ambient session.
    async fn graph_run(
        &self,
        title: &str,
        task: &str,
        profile: &str,
        resume: Option<&str>,
    ) -> Result<Value> {
        let mut payload = json!({"title": title, "task": task, "profile": profile});
        if let Some(r) = resume {
            payload["resume"] = json!(r);
        }
        if let Some(session) = self.state.borrow().graph_session.as_deref() {
            payload["session"] = json!(session);
        }
        services::call(
            Some(unipi_kit::suite::pkg::SUBAGENTS.id),
            point::SUBAGENTS_GRAPH_RUN,
            &payload,
            Some(Duration::from_secs(30)),
        )
        .await
    }

    /// A graph-run start failed (D6 §9.2). During a continue, an unrecoverable operator
    /// session spawns a **fresh** operator with the original instruction — once; a second
    /// unrecoverable loss leaves the work errored. Returns the work id when the fresh
    /// operator started, so `reconcile` can report it.
    async fn start_after_unrecoverable(
        &self,
        work_id: &str,
        profile: &str,
        error: &str,
    ) -> Option<String> {
        let retry_context = {
            let st = self.state.borrow();
            st.graph
                .open()
                .and_then(|g| g.work.iter().find(|w| w.work_id == work_id))
                .filter(|w| w.resume.is_some() && w.resume_operator_id.is_some())
                .is_some_and(|_| resume_unrecoverable(error))
        };
        if retry_context {
            let note = format!(
                "transient (crashed), operator session unrecoverable — continuing with a fresh operator: {error}"
            );
            if self
                .state
                .borrow_mut()
                .graph
                .fresh_spawn_once(work_id, &note, timers::now_ms())
            {
                let (title, task) = self.run_payload(work_id);
                match self.graph_run(&title, &task, profile, None).await {
                    Ok(v) if v.get("runId").and_then(Value::as_str).is_some() => {
                        let op = v["runId"].as_str().unwrap_or_default().to_owned();
                        let mut st = self.state.borrow_mut();
                        st.graph.started(work_id, Ok(op), timers::now_ms());
                        st.graph.set_ran(work_id, graph::RanOn::from_reply(&v));
                        drop(st);
                        self.save();
                        return Some(work_id.to_owned());
                    }
                    Ok(v) if v.get("retryable").and_then(Value::as_bool) == Some(true) => {
                        // Capacity: the fresh start re-runs on the next reconcile pass
                        // (retryable start errors already arm GRAPH_RETRY_TIMER there).
                        self.save();
                        timers::set(GRAPH_RETRY_TIMER, Duration::from_secs(5), false);
                        return None;
                    }
                    outcome => {
                        let e = match outcome {
                            Ok(v) => v
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("could not start")
                                .to_owned(),
                            Err(e) => e.to_string(),
                        };
                        self.fail_start(work_id, &e);
                        return None;
                    }
                }
            }
            // Second unrecoverable loss.
            self.fail_start(
                work_id,
                &format!("transient (crashed), operator session lost twice: {error}"),
            );
            return None;
        }
        self.fail_start(work_id, error);
        None
    }

    /// A refused start leaves the work `errored` and notifies (D6 §9.3).
    fn fail_start(&self, work_id: &str, error: &str) {
        self.state
            .borrow_mut()
            .graph
            .started(work_id, Err(error.to_owned()), timers::now_ms());
        self.save();
        self.notify_work(work_id);
    }

    /// User notification once per errored (or failed) work item — `ui.fire(notify)` +
    /// `ui.os-notify` (both land only when a frontend is attached; errors ignored) and a
    /// sticky bus publish for `unipi.notify` / the app. Errored work says it is not
    /// failed and can be retried.
    fn notify_work(&self, work_id: &str) {
        let Some((graph_id, status, error)) = (|| {
            let st = self.state.borrow();
            let g = st.graph.current.as_ref()?;
            let w = g.work.iter().find(|w| w.work_id == work_id)?;
            Some((
                g.graph_id.clone(),
                w.status.clone(),
                w.failure.clone().unwrap_or_else(|| "error".into()),
            ))
        })() else {
            return;
        };
        let (title, message, topic) = if status == "errored" {
            (
                "Agent graph work stopped on an error",
                format!(
                    "Agent graph work {work_id} stopped on an error: {error}. It is not failed — its session is kept; the lead was woken to continue it, or press Retry."
                ),
                WORK_ERRORED_TOPIC,
            )
        } else {
            (
                "Agent graph work failed",
                format!(
                    "Agent graph work {work_id} failed: {error}. Downstream work is blocked; the operator was nudged to replace, skip or re-plan."
                ),
                WORK_FAILED_TOPIC,
            )
        };
        let _ = ui::notify(&message, "warning");
        let _ = ui::os_notify(title, &message, "normal", None);
        // Sticky by name (unipi-kit's sticky table covers its own topics only).
        let _ = oino_extension::bus::publish(
            topic,
            &json!({"graphId": graph_id, "workId": work_id, "status": status, "error": error}),
            true,
        );
    }

    async fn run_done(&self, data: &Value, cx: &Ctx) {
        if data.get("runId").and_then(Value::as_str).is_none() {
            return;
        }
        // The run-done payload names the owner session the run was recorded under;
        // deliveries can carry a stale nested-session context (E-02), so prefer the
        // declared owner for the graph runs this settle is about to start.
        if let Some(owner) = data
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.state.borrow_mut().graph_session = Some(owner.to_owned());
        }
        if self.apply_run_done(data) == graph::SettleOutcome::Unknown {
            // A worker that finished before its start was recorded (the run-done raced
            // the `subagents.graph-run` reply): replay it once the start lands.
            let mut st = self.state.borrow_mut();
            if st.graph.open().is_some() {
                if st.early_done.len() >= EARLY_DONE_MAX {
                    st.early_done.remove(0);
                }
                st.early_done.push(data.clone());
            }
            return;
        }
        self.save();
        self.reconcile(cx).await;
        timers::set(GRAPH_WAKE_TIMER, Duration::from_millis(400), false);
    }

    /// Settles one run-done payload: arms the auto-continue timer or notifies an
    /// errored item. No reconcile (the caller starts what became runnable).
    fn apply_run_done(&self, data: &Value) -> graph::SettleOutcome {
        let Some(run) = data.get("runId").and_then(Value::as_str) else {
            return graph::SettleOutcome::Unknown;
        };
        let r = data.get("result").cloned().unwrap_or_default();
        let error = r.get("error").and_then(Value::as_str);
        let crashed = worker_crashed(error);
        let mut status = r.get("status").and_then(Value::as_str).unwrap_or("failed");
        // A run cancelled because its session went away (host shutdown / restart) was
        // interrupted, not stopped by anyone: it errors (and auto-continues).
        if status == "cancelled" && crashed {
            status = "failed";
        }
        let outcome = self.state.borrow_mut().graph.settle(
            run,
            status,
            r.get("text").and_then(Value::as_str).unwrap_or_default(),
            error,
            u32::try_from(r.get("toolCalls").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0),
            timers::now_ms(),
            crashed,
        );
        match &outcome {
            // Auto-continue: arm the durable retry timer (the schedule also persists in
            // graph.json, so a dead timer is re-armed on the next bind).
            graph::SettleOutcome::Retried { work_id, delay_ms } => timers::set(
                &format!("{RETRY_TIMER_PREFIX}{work_id}"),
                Duration::from_millis(*delay_ms),
                false,
            ),
            // Errored: notify once; dependents wait and the checkpoint wakes the
            // supervisor with the error text.
            graph::SettleOutcome::Settled { work_id, status } if status == "errored" => {
                self.save();
                self.notify_work(work_id);
            }
            graph::SettleOutcome::Settled { .. } | graph::SettleOutcome::Unknown => {}
        }
        outcome
    }

    /// Replays early run-dones whose start is now recorded (see `State::early_done`).
    fn replay_early_done(&self) {
        let ready: Vec<Value> = {
            let mut st = self.state.borrow_mut();
            if st.early_done.is_empty() {
                return;
            }
            let running: Vec<String> = st
                .graph
                .open()
                .map(|g| {
                    g.work
                        .iter()
                        .filter(|w| w.status == "running")
                        .filter_map(|w| w.operator_id.clone())
                        .collect()
                })
                .unwrap_or_default();
            let (ready, keep): (Vec<Value>, Vec<Value>) = std::mem::take(&mut st.early_done)
                .into_iter()
                .partition(|d| {
                    d.get("runId")
                        .and_then(Value::as_str)
                        .is_some_and(|r| running.iter().any(|o| o == r))
                });
            st.early_done = keep;
            ready
        };
        if ready.is_empty() {
            return;
        }
        for d in &ready {
            self.apply_run_done(d);
        }
        self.save();
        // Start what became runnable and wake the supervisor (timers: no re-entry).
        timers::set(GRAPH_RETRY_TIMER, Duration::from_millis(50), false);
        timers::set(GRAPH_WAKE_TIMER, Duration::from_millis(400), false);
    }

    async fn graph_wake(&self) {
        let cp = {
            let mut st = self.state.borrow_mut();
            let mut reported = std::mem::take(&mut st.reported_attention);
            let cp = st.graph.checkpoint(&mut reported, timers::now_ms());
            st.reported_attention = reported;
            cp
        };
        if let Some((_, t)) = cp {
            self.save();
            self.send(&t, "Graph checkpoint").await;
        }
    }

    /// The user's Retry on errored graph work (app button through [`GRAPH_SERVICE`],
    /// `/graph retry <work id>`): the same resume as the lead's `continue`, recorded as
    /// `by: user`; starts it now.
    async fn user_retry(
        &self,
        graph_id: Option<&str>,
        work_id: &str,
        note: Option<&str>,
        cx: &Ctx,
    ) -> std::result::Result<String, LhError> {
        let id = self.state.borrow_mut().graph.continue_work(
            graph_id,
            work_id,
            note,
            By::User,
            timers::now_ms(),
        )?;
        // The lead is not re-woken for an attention set it already saw.
        self.state
            .borrow_mut()
            .reported_attention
            .retain(|a| a != &id);
        self.save();
        self.reconcile(cx).await;
        Ok(id)
    }

    /// The user's Retry on an errored goal / ralph loop: clears the error and sends the
    /// continue prompt.
    async fn goal_retry(&self, note: Option<&str>) -> std::result::Result<(), LhError> {
        let error =
            self.state
                .borrow_mut()
                .machine
                .continue_errored(By::User, note, timers::now_ms())?;
        self.save();
        let prompt = goal::continue_prompt(&error, note);
        self.state.borrow_mut().machine.record_pending(&prompt, 0);
        self.send(&prompt, "Continue").await;
        Ok(())
    }

    /// The last assistant message's error text (a turn that ended on an error).
    fn last_error() -> Option<String> {
        session::query(&session::EntryQuery::messages(&["assistant"]).last(1))
            .ok()?
            .last()?
            .get("message")?
            .get("errorMessage")?
            .as_str()
            .map(|e| e.chars().take(4_000).collect())
    }

    /// A goal / ralph turn ended on an error (`agent_before_settle` outcome `error`): the
    /// turn does not count; a transient error auto-continues (bounded, backoff), else
    /// the goal waits as errored for a continue. Returns whether it was handled.
    fn lead_turn_error(&self) -> bool {
        let driving = {
            let st = self.state.borrow();
            st.machine.active().is_some()
                && st
                    .owners
                    .active
                    .as_ref()
                    .is_some_and(|o| matches!(o.kind, OwnerKind::Goal | OwnerKind::RalphLoop))
        };
        if !driving {
            return false;
        }
        let error = Self::last_error().unwrap_or_else(|| "the turn ended with an error".into());
        let class = graph::classify_failure(false, &error);
        let (transient, retry_after) = match class {
            graph::FailureClass::Transient { retry_after_ms } => (true, retry_after_ms),
            graph::FailureClass::Permanent => (false, None),
        };
        let budget = self.settings().auto_continue;
        let step = {
            let mut st = self.state.borrow_mut();
            st.proposal = None;
            st.stash = None;
            st.machine
                .turn_error(&error, transient, retry_after, budget, timers::now_ms())
        };
        match step {
            goal::ErrorStep::Ignored => return false,
            goal::ErrorStep::AutoContinue { delay_ms } => {
                timers::set(GOAL_CONTINUE_TIMER, Duration::from_millis(delay_ms), false);
            }
            goal::ErrorStep::Errored => {
                let msg = format!(
                    "The goal stopped on an error: {}. It is not failed — /goal continue (or Retry in the app) resumes it.",
                    error.chars().take(300).collect::<String>()
                );
                let _ = ui::notify(&msg, "warning");
                let _ = ui::os_notify("Goal stopped on an error", &msg, "normal", None);
            }
        }
        self.save();
        true
    }

    // ── commands ───────────────────────────────────────────────────────────

    fn stop_owner(&self, kind: Option<OwnerKind>, session: &str) -> Option<String> {
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
            st.graph.abandon(reason, By::User, timers::now_ms());
        }
        if active.kind == OwnerKind::RalphLoop
            && let Some(l) = st.ralph.as_mut()
        {
            l.status = "complete".into();
        }
        st.stash = None;
        st.session_override.remove(session);
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
            match self.ralph_start(name, &content.replace("\\n", "\n"), cx.session_id()) {
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
            let stopped = self.stop_owner(owner_kind, cx.session_id());
            notify(&if stopped.is_some() {
                format!("{} stopped.", def.label())
            } else {
                format!("No active {}.", def.label().to_lowercase())
            });
            return;
        }
        // The user's Retry: `/graph retry <work id> [note]` (errored graph / swarm work)
        // and `/goal continue [note]` / `/ralph continue [note]` (an errored loop).
        if matches!(mode, Mode::Graph | Mode::Swarm) && sub == "retry" {
            let Some(id) = parts.get(1).copied() else {
                notify(&format!("Usage: /{} retry <work id> [note]", mode.as_str()));
                return;
            };
            let note = args
                .trim_start()
                .strip_prefix("retry")
                .unwrap_or_default()
                .trim_start()
                .strip_prefix(id)
                .map(str::trim)
                .filter(|n| !n.is_empty());
            match self.user_retry(None, id, note, &cx.cx).await {
                Ok(id) => notify(&format!("Retrying {id} on its own session.")),
                Err(e) => notify(&format!("Not retried: {e} ({}).", e.code.as_str())),
            }
            return;
        }
        if matches!(mode, Mode::Goal | Mode::Ralph) && sub == "continue" {
            let note = args
                .trim_start()
                .strip_prefix("continue")
                .map(str::trim)
                .filter(|n| !n.is_empty());
            match self.goal_retry(note).await {
                Ok(()) => notify("Continuing from where it stopped."),
                Err(e) => notify(&format!("Not continued: {e} ({}).", e.code.as_str())),
            }
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
                let current = st.turn_modes.get(cx.session_id()).map_or_else(
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
            // D-2: a mode-command status is a chat row (unipi prints these in the
            // transcript). A long `notify` would open the blocking result-card modal,
            // which owns the keyboard and swallows the next typed command.
            respond(&msg).await;
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
                    if let Some(o) = r.as_ref().map(|o| o.kind.mode()) {
                        st.pending_explicit.insert(cx.session_id().to_owned(), o);
                    }
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
            st.pending_explicit.insert(cx.session_id().to_owned(), mode);
            st.session_override.insert(cx.session_id().to_owned(), mode);
            // [graph-mode] the status topic (the app's mode chip) shows the switch at once —
            // also while the switch waits for a running turn — instead of at the turn's start.
            st.display = Some(mode);
            apply_mode_tools(mode);
        }
        self.save();
        self.publish();
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

    /// [`GRAPH_SERVICE`] — the app's graph data and Retry. Payload `{op, …}`:
    /// - `snapshot {graphId?}` → `{ok, graph}` (the current graph, else the newest earlier
    ///   one; `graph: null` when none) — see `GraphFile::snapshot`.
    /// - `history {}` → `{ok, graphs}` summary rows, current first.
    /// - `output {workId, graphId?}` → `{ok, output}`: the stored result / partial output
    ///   (≤ 64 KB; earlier graphs ≤ 16 KB) and the error.
    /// - `retry {workId, graphId?, note?}` → `{ok, workId}`: the user's Retry on errored
    ///   graph work (same as the lead's `continue`, `by: user`).
    /// - `retry {target: "goal", note?}` → `{ok}`: the user's Retry on an errored goal /
    ///   ralph loop.
    ///
    /// Refusals are `{ok:false, code, reason}` with a [`LhCode`].
    async fn graph_service(&self, p: &Value, cx: &Ctx) -> Value {
        let s = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        match s("op").unwrap_or_default() {
            "snapshot" => {
                let live = Self::live_operators().await;
                let st = self.state.borrow();
                let g = match s("graphId") {
                    Some(id) => match st.graph.graph(id) {
                        Some(g) => Some(g),
                        None => {
                            return LhError {
                                code: LhCode::GraphNotFound,
                                message: format!("no graph {id}"),
                            }
                            .to_json();
                        }
                    },
                    None => st
                        .graph
                        .current
                        .as_ref()
                        .or_else(|| st.graph.history.first()),
                };
                json!({"ok": true, "graph": g.map(|g| st.graph.snapshot_with(g, &live))})
            }
            // The app's Stop on an orphaned item: work the graph moved on from (stopped,
            // replaced, settled) whose worker still runs.
            "stop_operator" => match s("workId") {
                None => LhError {
                    code: LhCode::RequestInvalid,
                    message: "workId is required".into(),
                }
                .to_json(),
                Some(id) => {
                    let op = {
                        let st = self.state.borrow();
                        st.graph.current.as_ref().and_then(|g| {
                            g.work
                                .iter()
                                .find(|w| w.work_id == id)
                                .map(|w| (w.status.clone(), w.operator_id.clone()))
                        })
                    };
                    match op {
                        None => LhError {
                            code: LhCode::WorkNotFound,
                            message: format!("no graph work {id}"),
                        }
                        .to_json(),
                        Some((_, None)) => LhError {
                            code: LhCode::RequestInvalid,
                            message: format!("{id} has no worker"),
                        }
                        .to_json(),
                        Some((status, Some(_))) if status == "running" => LhError {
                            code: LhCode::RequestInvalid,
                            message: format!(
                                "{id} is running in the graph: the lead stops it (update_agent_graph stop)"
                            ),
                        }
                        .to_json(),
                        Some((_, Some(op))) => {
                            let r = services::call(
                                Some(unipi_kit::suite::pkg::SUBAGENTS.id),
                                point::SUBAGENTS_CANCEL,
                                &json!({"id": op}),
                                None,
                            )
                            .await;
                            let cancelled = r
                                .ok()
                                .and_then(|v| v.get("cancelled").and_then(Value::as_bool))
                                .unwrap_or(false);
                            json!({"ok": true, "workId": id, "operatorId": op, "cancelled": cancelled})
                        }
                    }
                }
            },
            "history" => json!({"ok": true, "graphs": self.state.borrow().graph.history_list()}),
            "output" => match s("workId") {
                None => LhError {
                    code: LhCode::RequestInvalid,
                    message: "workId is required".into(),
                }
                .to_json(),
                Some(id) => match self.state.borrow().graph.read_result(id, s("graphId")) {
                    None => LhError {
                        code: LhCode::WorkNotFound,
                        message: format!("no graph work or result {id}"),
                    }
                    .to_json(),
                    Some((gid, w)) => {
                        json!({"ok": true, "output": {"graphId": gid, "workId": w.work_id, "status": w.status, "resultId": w.result_id, "operatorId": w.operator_id, "instruction": w.instruction, "text": w.result, "error": w.failure, "truncated": w.result_truncated}})
                    }
                },
            },
            "retry" if s("target") == Some("goal") => match self.goal_retry(s("note")).await {
                Ok(()) => json!({"ok": true}),
                Err(e) => e.to_json(),
            },
            "retry" => match s("workId") {
                None => LhError {
                    code: LhCode::RequestInvalid,
                    message: "workId (or target:\"goal\") is required".into(),
                }
                .to_json(),
                Some(id) => match self.user_retry(s("graphId"), id, s("note"), cx).await {
                    Ok(id) => json!({"ok": true, "workId": id}),
                    Err(e) => e.to_json(),
                },
            },
            op => LhError {
                code: LhCode::RequestInvalid,
                message: format!(
                    "unknown op \"{op}\" (snapshot, history, output, retry, stop_operator)"
                ),
            }
            .to_json(),
        }
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
                let _ = ui::status("lh-progress", None::<&str>);
                let _ = ui::notify(
                    "Goal progress estimate failed (model unavailable or unparseable reply).",
                    "info",
                );
            }
        }
    }

    /// `/visualize-progress` (mermaid source + text fallback; no PNG in v1). The open
    /// graph, else the last one (history[0]); each node carries its profile and, once it
    /// ran, the model + thinking from the subagents run record. The structured nodes go
    /// in the reserved entry's `params` (`{graph, nodes}`); the fallback tree keeps the
    /// `- <work> [<status>] <instruction>` lines (plus an indented `ran on` line).
    fn visualize(&self) {
        let st = self.state.borrow();
        let mut mermaid = vec!["flowchart TD".to_owned()];
        let mut textv = Vec::new();
        let mut params = json!({"v": 1});
        if let Some(g) = st
            .graph
            .current
            .as_ref()
            .or_else(|| st.graph.history.first())
        {
            let v = st.graph.visual(g);
            mermaid.extend(v.mermaid);
            textv.extend(v.lines);
            let state = if g.finish.is_some() {
                "finished"
            } else if g.abandoned.is_some() {
                "stopped"
            } else {
                "open"
            };
            params["graph"] = json!({"id": g.graph_id, "mode": g.mode, "task": g.task, "state": state, "revision": g.revision});
            let from = g.events.len().saturating_sub(VISUAL_GRAPH_EVENTS);
            params["graph"]["events"] = json!(g.events[from..]);
            params["nodes"] = json!(v.nodes);
            // Earlier graphs as data (summary rows; a row's full snapshot comes from
            // `long-horizon.graph` `{op:"snapshot", graphId}`).
            params["history"] = st.graph.history_list();
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
        let spec: Node = Layout::default()
            .spacing(1)
            .children([
                Code::new(mermaid.join("\n")).language("mermaid").into(),
                Markdown::new(textv.join("\n")).into(),
            ])
            .into();
        // `unipi.long-horizon/visualize` is a reserved entry (§8): the app scrolls its Git pane
        // to the long-horizon section; the tree above is the fallback for other frontends.
        let Ok(id) = ui::Panel::new("unipi.long-horizon/visualize")
            .placement(ui::Placement::SidePane)
            .title("Long-horizon progress")
            .icon(ui::Icon::Chart)
            .focus(true)
            .close_on_escape(true)
            .params(params)
            .open()
        else {
            return;
        };
        let _ = ui::Panel::update(&id).fallback(spec).send();
    }

    async fn runner(&'static self, name: &str, args: &str, cx: &CommandCtx) -> Option<Value> {
        let session = cx.session_id();
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
                        self.state
                            .borrow_mut()
                            .pending_explicit
                            .insert(session.to_owned(), Mode::Goal);
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
                        st.pending_explicit.insert(session.to_owned(), Mode::Goal);
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
                match self.stop_owner(Some(OwnerKind::Goal), session) {
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
                            st.pending_explicit.insert(session.to_owned(), mode);
                            st.session_override.insert(session.to_owned(), mode);
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
                    match self.ralph_start(&n, &c, session) {
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
                            let kind = if m == GraphMode::Swarm {
                                OwnerKind::Swarm
                            } else {
                                OwnerKind::Graph
                            };
                            if self.stop_owner(Some(kind), session).is_none() {
                                self.state.borrow_mut().graph.abandon(
                                    "stopped(user_requested)",
                                    By::User,
                                    timers::now_ms(),
                                );
                                self.save();
                            }
                            json!({"ok": true})
                        }
                    }
                }
                "history" => {
                    json!({"ok": true, "runs": self.state.borrow().graph.history_json(), "graphs": self.state.borrow().graph.history_list()})
                }
                // Same path as the app's `long-horizon.graph` service.
                "snapshot" | "retry" => {
                    let mut p = a.clone();
                    p["op"] = json!(s("op"));
                    self.graph_service(&p, &cx.cx).await
                }
                op => json!({"ok": false, "reason": format!("unknown op \"{op}\"")}),
            },
            _ => return None,
        };
        self.save();
        Some(out)
    }

    async fn session_start(&self, ev: &Event, cx: &Ctx) {
        if cx.is_child() {
            // Worker/child sessions share this process and its one piece of extension
            // state: the parent's in-memory graph/owner state is the live truth (it can
            // hold changes not yet on disk), and recovery / retry-rearm / mode-tool sync
            // are parent duties. A child reload here wipes unsaved state mid-spawn.
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            st.loaded = false;
            st.graph_session = Some(cx.session_id().to_owned());
            if matches!(
                ev.get("reason").and_then(Value::as_str),
                Some("new" | "resume" | "fork")
            ) {
                // This session's own mode keys restart; other sessions keep theirs
                // (F-06: mode state is per chat, never a shared slot).
                st.session_override.remove(cx.session_id());
                st.pending_explicit.remove(cx.session_id());
                st.turn_modes.remove(cx.session_id());
                st.turns.remove(cx.session_id());
            }
        }
        self.load();
        let owner_mode = self
            .state
            .borrow()
            .owners
            .active
            .as_ref()
            .map_or(Mode::None, |o| o.kind.mode());
        apply_mode_tools(owner_mode);
        // Crash recovery: re-deliver the durable outbox (at most twice), else arm a recovery fragment.
        let redeliver = {
            let mut st = self.state.borrow_mut();
            st.display = Some(
                st.session_override
                    .get(cx.session_id())
                    .copied()
                    .unwrap_or(self.settings().default_mode),
            );
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
        // Restart: work that was running is interrupted. It resumes its own operator
        // shortly (by `host`, bounded; else errored) — the same continue path as an
        // auto-continue, so an interrupted graph never stalls (D6-03: work in flight at
        // quit used to stay queued forever).
        let errored_now = {
            let mut st = self.state.borrow_mut();
            let before: Vec<String> = st.graph.errored().into_iter().map(|e| e.0).collect();
            if st.graph.owned_by(cx.session_id()) {
                st.graph.interrupted(timers::now_ms());
            }
            st.graph
                .errored()
                .into_iter()
                .map(|e| e.0)
                .filter(|id| !before.contains(id))
                .collect::<Vec<_>>()
        };
        // The retry schedule is the file — on bind, overdue continues fire after a short
        // grace (subagents binds its services first) and future ones re-arm their timers.
        // Queued work that is ready (a start lost to the restart) gets the same grace.
        let start_soon = {
            let st = self.state.borrow();
            let now = timers::now_ms();
            let (due, future) = st.graph.retry_schedule(now);
            for (work_id, delay_ms) in future {
                timers::set(
                    &format!("{RETRY_TIMER_PREFIX}{work_id}"),
                    Duration::from_millis(delay_ms),
                    false,
                );
            }
            !due.is_empty() || !st.graph.runnable(now).is_empty()
        };
        self.save();
        for id in &errored_now {
            self.notify_work(id);
        }
        if start_soon {
            timers::set(
                GRAPH_RETRY_TIMER,
                Duration::from_millis(graph::RESTART_RESUME_DELAY_MS),
                false,
            );
        }
        if !errored_now.is_empty() {
            timers::set(GRAPH_WAKE_TIMER, Duration::from_millis(400), false);
        }
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
                    .argument_hint(match mode {
                        Mode::Ralph => {
                            "start <name> | stop | status | continue | resume | clear | <prompt>"
                        }
                        Mode::Goal => "<prompt> | status | continue | stop | resume | clear",
                        _ => {
                            "<prompt> | status | history | retry <work id> | stop | resume | clear"
                        }
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
            .run(|ext, _a, cx| async move {
                ext.load();
                ext.stop_owner(None, cx.session_id());
                {
                    let mut st = ext.state.borrow_mut();
                    let session = cx.session_id().to_owned();
                    st.pending_explicit.insert(session.clone(), Mode::None);
                    st.session_override.insert(session, Mode::None);
                    // `/regular` clears the mode outright: `publish` falls back to `display`
                    // when no owner is active, so leaving the last resolved mode there keeps
                    // the `Goal`/`Ralph` status chip alive after the stop.
                    st.display = Some(Mode::None);
                }
                apply_mode_tools(Mode::None);
                ext.save();
                // [graph-mode] the chip leaves now, not at the next turn's start.
                ext.publish();
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
                    .run_value(
                        move |ext, args, cx| async move { Ok(ext.runner(r, &args, &cx).await) },
                    ),
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
        // Graph work in flight wakes the lead at its checkpoints: the lead waits (Working)
        // while it runs, between waves and across a restart resume too.
        cx.wait_source(|ext, cx| async move {
            if cx.is_child() {
                return None;
            }
            ext.load();
            ext.state.borrow().graph.wait_reason(cx.session_id())
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
        cx.on(EventKind::ToolCall, |ext, ev, cx| async move {
            Ok(ext.tool_call(&ev, &cx))
        });
        cx.on(EventKind::ToolResult, |ext, ev, cx| async move {
            ext.tool_result(&ev, &cx).await;
            Ok(())
        });
        cx.on(EventKind::AgentBeforeSettle, |ext, ev, cx| async move {
            let session = cx.session_id().to_owned();
            if cx.is_child() {
                return Ok(());
            }
            let outcome = ev.get("outcome").and_then(Value::as_str);
            let aborted = outcome.is_some_and(|o| o != "completed");
            // A goal / ralph turn cut off by an error is `errored`, never a stall: it
            // auto-continues (bounded) or waits for /goal continue / the app's Retry.
            if outcome == Some("error") && ext.lead_turn_error() {
                return Ok(());
            }
            if outcome != Some("error") {
                // A turn that ran (the user typed on, or a continue landed) clears the
                // error and starts the auto-continue budget over.
                let continued = {
                    let mut st = ext.state.borrow_mut();
                    st.machine.turn_ok();
                    st.machine
                        .goal
                        .as_ref()
                        .is_some_and(|g| g.errored.is_some())
                        && st
                            .machine
                            .continue_errored(By::User, Some("the next turn ran"), timers::now_ms())
                            .is_ok()
                };
                if continued {
                    ext.save();
                }
            }
            ext.goal_turn_end(aborted, &session).await;
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
                Some(GOAL_CONTINUE_TIMER) => {
                    let pending = ext.state.borrow_mut().machine.take_continue_cause();
                    if let Some(cause) = pending {
                        let prompt = goal::continue_prompt(&cause, None);
                        ext.state.borrow_mut().machine.record_pending(&prompt, 0);
                        ext.save();
                        Lh::send_now(&prompt, "Continue").await;
                    }
                }
                Some(GRAPH_RETRY_TIMER) => {
                    ext.reconcile(&cx).await;
                }
                // D6 §9.2: a per-work transient retry came due — start it (the due
                // check is `runnable(now)`; a stale early fire re-arms itself).
                Some(id) if id.starts_with(RETRY_TIMER_PREFIX) => {
                    let work_id = &id[RETRY_TIMER_PREFIX.len()..];
                    let now = timers::now_ms();
                    let due = {
                        let st = ext.state.borrow();
                        st.graph
                            .open()
                            .and_then(|g| g.work.iter().find(|w| w.work_id == work_id))
                            .is_some_and(|w| {
                                w.status == "queued" && w.next_retry_at.is_some_and(|at| at <= now)
                            })
                    };
                    if due {
                        ext.reconcile(&cx).await;
                    } else {
                        let remaining = {
                            let st = ext.state.borrow();
                            st.graph
                                .open()
                                .and_then(|g| g.work.iter().find(|w| w.work_id == work_id))
                                .and_then(|w| w.next_retry_at)
                                .filter(|at| *at > now)
                                .map(|at| u64::try_from(at - now).unwrap_or(1))
                        };
                        if let Some(ms) = remaining {
                            timers::set(id, Duration::from_millis(ms), false);
                        }
                    }
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
        cx.on(EventKind::SessionStart, |ext, ev, scx| async move {
            ext.session_start(&ev, &scx).await;
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
            let auto_continue = ext.settings().auto_continue;
            ext.state.borrow_mut().graph.policy = graph::Policy { auto_continue };
            Ok(())
        });
        // The app's graph data + Retry (`_oino/service_call` → `long-horizon.graph`).
        cx.service(GRAPH_SERVICE, |ext, p, _caller, cx| async move {
            ext.load();
            Ok(ext.graph_service(&p, &cx).await)
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

    #[test]
    fn wp_x7_helpers() {
        // Worker-crash detection from subagents run-done outcomes.
        assert!(worker_crashed(Some("Subagent session ended")));
        assert!(worker_crashed(Some("session ended while it ran")));
        assert!(worker_crashed(Some(
            "interrupted — Oino exited while it ran"
        )));
        assert!(!worker_crashed(Some("HTTP 429 too many requests")));
        assert!(!worker_crashed(None));
        assert!(!worker_crashed(Some("boom")));
        // Unrecoverable operator sessions.
        assert!(resume_unrecoverable(
            "No subagent found for ab12 — can't resume."
        ));
        assert!(resume_unrecoverable(
            "failed to read session file: corrupt jsonl"
        ));
        assert!(resume_unrecoverable("session file unreadable"));
        assert!(!resume_unrecoverable(
            "Maximum 8 concurrent subagents running"
        ));
        assert!(!resume_unrecoverable("unknown subagent profile \"x\""));
        // D6 §9.2 timer ids.
        assert_eq!(format!("{RETRY_TIMER_PREFIX}scan-api"), "lh-retry-scan-api");
        assert_eq!(WORK_FAILED_TOPIC, "unipi:long-horizon:work-failed");
    }

    /// [graph-mode] P0: every mode command (`/goal`, `/ralph`, `/swarm`, `/graph`, and
    /// `/regular` = none) from every other mode enables exactly its own tools, those tools are
    /// registered by this extension (an unregistered name would be ignored by the host), and the
    /// graph tools the orchestration prompt names are among them.
    #[test]
    fn every_mode_command_switches_to_its_registered_tools() {
        let registered: Vec<&str> = tools::all().into_iter().map(|(n, ..)| n).collect();
        let builtins = ["read", "bash", "edit", "write"];
        let all = [
            Mode::None,
            Mode::Goal,
            Mode::Ralph,
            Mode::Swarm,
            Mode::Graph,
        ];
        for name in ["goal", "ralph", "swarm", "graph", "none"] {
            let mode = Mode::parse(name).unwrap();
            assert_eq!(mode.as_str(), name);
            for t in mode.control_tools() {
                assert!(registered.contains(t), "{name}: {t} not registered");
            }
            for from in all {
                let mut active: Vec<String> = builtins.iter().map(|s| (*s).to_owned()).collect();
                active.extend(from.control_tools().iter().map(|s| (*s).to_owned()));
                if let Some((enable, disable)) = modes::mode_tool_update(&active, mode) {
                    active.retain(|t| !disable.contains(t) || enable.contains(t));
                    for e in enable {
                        if !active.contains(&e) {
                            active.push(e);
                        }
                    }
                }
                for b in builtins {
                    assert!(active.iter().any(|a| a == b), "{from:?}->{name}: {b} lost");
                }
                for t in modes::all_mode_tools() {
                    assert_eq!(
                        active.iter().any(|a| a == t),
                        mode.control_tools().contains(&t),
                        "{from:?}->{name}: {t}"
                    );
                }
            }
        }
        for t in [
            "update_agent_graph",
            "view_agent_graph",
            "yield_agent_graph",
            "agent_output",
            "agent_swarm_status",
        ] {
            assert!(Mode::Graph.control_tools().contains(&t), "{t}");
            assert!(
                modes::mode_fragment(Mode::Graph, None, None).contains(t),
                "the Graph prompt names {t}"
            );
        }
        assert!(modes::mode_fragment(Mode::Graph, None, None).contains("Mode: Graph"));
    }
}
