//! `unipi.watchdog` — jev watchdog for long-running tool calls and background tasks, plus the
//! opt-in session caps that replace Oino `budget` (unipi `packages/watchdog`, unipi-port §3.10).
//!
//! | unipi | here |
//! |---|---|
//! | heartbeat (≤ 15 s), due check (`firstCheckMin`, `0.9 × intervalMin`) | timer `watchdog-heartbeat`, [`Watchdog::tick`] |
//! | bash: `expect`, process sample, `kind/status/stop` jev questions, [`decide::decide_bash`] | `control.running-bash` pid + `process-stats`/`process-tree` ([`sample`]) |
//! | bg tasks / other tools: `status/persistent`, [`decide::evaluate_tick`] | `tasks.list`/`tasks.logs`; tool execution events |
//! | `background` action (adopt) | `control.bash-detach` |
//! | `kill` action (`⚠ Killed by unipi watchdog …`, `isError`) | detach + `tasks.kill`, `tool_result` rewrite |
//! | warnings at the next turn (`unipi-watchdog` message) | `before_agent_start` message with provenance |
//! | `otherTools: abort-turn` | `control.abort` + follow-up user message |
//! | `/unipi:bg-detach [reason]` | command |
//! | status `watchdog: N` | `setStatus` |
//! | settings `watchdog` + `decisionModel` | namespace ([`settings`]) |
//! | Oino `budget` caps | `sessionCaps.*` + flags `--max-turns`, `--max-cost`, `--max-wall`, `--max-tool-errors` ([`caps`]) |

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests assert by unwrapping"
    )
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

pub mod caps;
pub mod decide;
pub mod prompt;
pub mod sample;
pub mod settings;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use oino_extension::command::{Flag, get_flag};
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::session::{Provenance, UserMessageOptions};
use unipi_kit::jev;
use unipi_kit::settings::Store;

use crate::caps::{Caps, RunState, Verdict};
use crate::decide::{BashCheck, Expect, evaluate_tick, js_round};
use crate::sample::{ProcSample, Stat};
use crate::settings::{Action, EXTENSION_ID, NAMESPACE, OtherTools, Settings};

/// Heartbeat timer id.
const HEARTBEAT: &str = "watchdog-heartbeat";
/// Process sample window.
const SAMPLE_WINDOW_MS: u64 = 5_000;
/// Tools that are quick or human-interactive — never watched.
pub const NEVER_WATCH: &[&str] = &[
    "read",
    "write",
    "edit",
    "grep",
    "find",
    "ls",
    "ask_user",
    "read_subagent",
    "bg_kill",
    "bg_tasks",
];

#[derive(Clone, Debug, Default)]
struct Tracked {
    tool_call_id: String,
    tool_name: String,
    display: String,
    started_at: i64,
    last_change_at: i64,
    last_tail: String,
    total_bytes: u64,
    prev_idle: bool,
    prev_cumulative_io: Option<u64>,
    stop_streak: u32,
    prev_age: Option<f64>,
    prev_check_at: Option<i64>,
    expect: Option<Expect>,
}

#[derive(Clone, Debug)]
struct Kill {
    duration_ms: i64,
    reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemKind {
    Bash,
    Bg,
    Other,
}

#[derive(Clone, Debug)]
struct Item {
    key: String,
    kind: ItemKind,
    tool_name: String,
    command: String,
    started_at: i64,
    since_last_output_sec: Option<i64>,
    tail: String,
}

#[derive(Default)]
struct State {
    bash: HashMap<String, Tracked>,
    other: HashMap<String, Tracked>,
    streaks: HashMap<String, u32>,
    kills: HashMap<String, Kill>,
    pending: Vec<String>,
    busy: bool,
    tails: HashMap<String, String>,
    changes: HashMap<String, i64>,
    checked: HashMap<String, i64>,
    warned: HashSet<String>,
    run: RunState,
}

/// The extension.
#[derive(Default)]
pub struct Watchdog {
    settings: Store,
    sessions: SessionMap<State>,
}

fn key_id(key: &str) -> &str {
    key.split_once(':').map_or(key, |(_, id)| id)
}

/// `details.oinoProvenance` for harness-tagged results/messages.
fn provenance(title: &str, synopsis: &str, delivery: &str) -> Value {
    json!({"extensionId": EXTENSION_ID, "source": "Watchdog", "title": title, "synopsis": synopsis, "severity": "warning", "delivery": delivery})
}

/// unipi kill warning prefix.
#[must_use]
pub fn kill_warning(duration_ms: i64, reason: &str) -> String {
    #[allow(clippy::cast_precision_loss, reason = "seconds")]
    let secs = js_round(duration_ms as f64 / 1000.0);
    format!(
        "⚠ Killed by unipi watchdog after {secs}s: jev judged it {reason}. Do not blindly re-run; investigate or change approach."
    )
}

/// unipi `queueWarn` text.
#[must_use]
pub fn warn_text(tool: &str, status: &str, confidence: f64, streak: u32, reason: &str) -> String {
    format!(
        "⚠ Watchdog: \"{tool}\" looks {status} (confidence {confidence:.2}, {streak} consecutive checks): {reason}."
    )
}

impl Watchdog {
    fn config(&self) -> Settings {
        Settings::load(&self.settings.get(NAMESPACE))
    }

    fn debug(&self, line: &str) {
        if self.settings.get(NAMESPACE).bool("debug", false) {
            host::log(host::LogLevel::Debug, line);
        }
    }

    /// The decision model for this module (`decisionModel` = inherit/custom over the shared one).
    fn jev_settings(&self) -> jev::Settings {
        let mut all = control::settings()
            .ok()
            .and_then(|s| s.get("extensionSettings").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        all[NAMESPACE] = self.settings.get(NAMESPACE).0;
        jev::Settings::resolve(&all, Some(NAMESPACE))
    }

    fn start_timer(&self) {
        let s = self.config();
        if s.enabled {
            timers::set(HEARTBEAT, Duration::from_millis(s.heartbeat_ms()), true);
        } else {
            timers::cancel(HEARTBEAT);
        }
    }

    // ── tracking ────────────────────────────────────────────────────────────

    fn on_tool_start(&self, ev: &Event, cx: &Ctx) {
        let name = ev
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if NEVER_WATCH.contains(&name) {
            return;
        }
        let id = ev
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let now = timers::now_ms();
        let t = Tracked {
            tool_call_id: id.to_owned(),
            tool_name: name.to_owned(),
            display: prompt::summarize_args(ev.get("args").unwrap_or(&Value::Null)),
            started_at: now,
            last_change_at: now,
            ..Tracked::default()
        };
        self.sessions.with(cx.session_id(), |s| {
            if name == "bash" {
                s.bash.insert(id.to_owned(), t);
            } else {
                s.other.insert(id.to_owned(), t);
            }
        });
    }

    fn on_tool_update(&self, ev: &Event, cx: &Ctx) {
        let id = ev
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tail = prompt::extract_text(ev.get("partialResult").unwrap_or(&Value::Null));
        let now = timers::now_ms();
        self.sessions.with(cx.session_id(), |s| {
            let Some(t) = s.bash.get_mut(id).or_else(|| s.other.get_mut(id)) else {
                return;
            };
            t.total_bytes = t.total_bytes.max(tail.len() as u64);
            if !tail.is_empty() && tail != t.last_tail {
                t.last_change_at = now;
                t.last_tail = tail;
            }
        });
    }

    fn on_tool_end(&self, ev: &Event, cx: &Ctx) {
        let id = ev
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.sessions.with(cx.session_id(), |s| {
            s.bash.remove(id);
            s.other.remove(id);
            s.streaks.remove(id);
            s.warned
                .retain(|k| !(k.starts_with(&format!("{id}:")) || k.ends_with(&format!(":{id}"))));
        });
    }

    /// unipi `tool_result` rewrite of a watchdog-killed bash call.
    fn on_tool_result(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let id = ev
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kill = self
            .sessions
            .with(cx.session_id(), |s| s.kills.remove(id))?;
        let original = ev
            .get("content")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let warning = kill_warning(kill.duration_ms, &kill.reason);
        let mut details = ev
            .get("details")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        details["oinoProvenance"] = provenance(
            "Kill warning",
            "Killed after timeout — do not blindly re-run",
            "boundary",
        );
        Some(json!({
            "content": [{"type": "text", "text": format!("{warning}\n\n{original}")}],
            "isError": true,
            "details": details,
        }))
    }

    fn on_before_agent_start(&self, cx: &Ctx) -> Option<Value> {
        let pending = self
            .sessions
            .with(cx.session_id(), |s| std::mem::take(&mut s.pending));
        if pending.is_empty() {
            return None;
        }
        Some(json!({"message": {
            "customType": "unipi-watchdog",
            "content": pending.join("\n\n"),
            "display": true,
            "details": {"oinoProvenance": provenance("Watchdog warnings", "Drained pending warnings", "before_agent_start")},
        }}))
    }

    // ── tick ────────────────────────────────────────────────────────────────

    fn gather(&self, s: &Settings, cx: &Ctx) -> Vec<Item> {
        let now = timers::now_ms();
        let mut items = Vec::new();
        self.sessions.with(cx.session_id(), |st| {
            if s.watch_bash {
                for t in st.bash.values() {
                    let key = format!("bash:{}", t.tool_call_id);
                    let last_change = st.changes.get(&key).copied().unwrap_or(t.last_change_at);
                    items.push(Item {
                        key,
                        kind: ItemKind::Bash,
                        tool_name: "bash".into(),
                        command: t.display.chars().take(500).collect(),
                        started_at: t.started_at,
                        since_last_output_sec: Some(round_sec(now - last_change)),
                        tail: prompt::last_chars(&t.last_tail, 3000),
                    });
                }
            }
            if s.other_tools != OtherTools::Off {
                for t in st.other.values() {
                    items.push(Item {
                        key: format!("other:{}", t.tool_call_id),
                        kind: ItemKind::Other,
                        tool_name: t.tool_name.clone(),
                        command: t.display.chars().take(500).collect(),
                        started_at: t.started_at,
                        since_last_output_sec: Some(round_sec(now - t.last_change_at)),
                        tail: prompt::last_chars(&t.last_tail, 3000),
                    });
                }
            }
        });
        if s.watch_bg_tasks {
            for t in tasks::list().unwrap_or_default() {
                let get = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or_default();
                let adopted = !get("owner").is_empty() && get("owner") != "unipi.background-tasks";
                let trigger = t
                    .get("triggerOnCompletion")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if get("state") != "running" || adopted || !trigger {
                    continue;
                }
                let id = get("id").to_owned();
                let logs = tasks::logs(&id, 16 * 1024, true).unwrap_or_default();
                let tail = unipi_last_lines(&logs, 3);
                items.push(Item {
                    key: format!("bg:{id}"),
                    kind: ItemKind::Bg,
                    tool_name: "bg_run".into(),
                    command: get("command").chars().take(500).collect(),
                    started_at: t.get("startedAt").and_then(Value::as_i64).unwrap_or(now),
                    since_last_output_sec: None,
                    tail: prompt::last_chars(&tail, 3000),
                });
            }
        }
        items
    }

    async fn sample(pid: u32, command: &str) -> Option<ProcSample> {
        let read = |pids: &[u32]| -> Vec<Stat> {
            pids.iter()
                .filter_map(|p| process::stats(*p).ok())
                .map(|s| Stat {
                    pid: s.pid,
                    ppid: s.ppid,
                    state: s.state.chars().next().unwrap_or('?'),
                    cpu_ms: s.cpu_ms,
                    read_bytes: s.read_bytes,
                    write_bytes: s.write_bytes,
                    start_time_ms: s.start_time_ms,
                })
                .collect()
        };
        let members = |pid: u32| {
            let mut all = vec![pid];
            all.extend(process::tree(pid).unwrap_or_default());
            all
        };
        let before = read(&members(pid));
        timers::sleep(Duration::from_millis(SAMPLE_WINDOW_MS)).await;
        let after = read(&members(pid));
        let mut sample = sample::from_stats(&before, &after, SAMPLE_WINDOW_MS)?;
        sample.sleeper_hint(command);
        Some(sample)
    }

    /// One heartbeat.
    async fn tick(&self, cx: &Ctx) {
        let busy = self
            .sessions
            .with(cx.session_id(), |s| std::mem::replace(&mut s.busy, true));
        if busy {
            return;
        }
        self.tick_inner(cx).await;
        self.sessions.with(cx.session_id(), |s| s.busy = false);
    }

    #[allow(clippy::too_many_lines, reason = "one pass of unipi's tick()")]
    async fn tick_inner(&self, cx: &Ctx) {
        let s = self.config();
        if !s.enabled {
            timers::cancel(HEARTBEAT);
            return;
        }
        let now = timers::now_ms();
        #[allow(clippy::cast_possible_truncation, reason = "ms")]
        let first_ms = (s.first_check_min * 60_000.0) as i64;
        let items: Vec<Item> = self
            .gather(&s, cx)
            .into_iter()
            .filter(|i| now - i.started_at >= first_ms)
            .collect();
        let status = (!items.is_empty()).then(|| format!("watchdog: {}", items.len()));
        let _ = ui::status("watchdog", status.as_deref());
        if items.is_empty() {
            return;
        }
        let jev_settings = self.jev_settings();
        #[allow(clippy::cast_possible_truncation, reason = "ms")]
        let due_ms = (s.interval_min * 60_000.0 * 0.9) as i64;
        for item in items {
            let (last_checked, previous_streak, previous_tail) =
                self.sessions.with(cx.session_id(), |st| {
                    (
                        st.checked.get(&item.key).copied().unwrap_or(0),
                        st.streaks.get(&item.key).copied().unwrap_or(0),
                        st.tails.get(&item.key).cloned(),
                    )
                });
            if now - last_checked < due_ms {
                continue;
            }
            let output_changed = previous_tail.as_ref().is_some_and(|p| *p != item.tail);
            let last_change = self.sessions.with(cx.session_id(), |st| {
                if previous_tail.is_none() || output_changed {
                    st.changes.insert(item.key.clone(), now);
                }
                st.changes
                    .get(&item.key)
                    .copied()
                    .unwrap_or(item.started_at)
            });
            let since_change = round_sec(now - last_change);
            let since_last_output = item
                .since_last_output_sec
                .map_or(since_change, |s| s.min(since_change));

            let mut bash_reason = String::new();
            let decision = if item.kind == ItemKind::Bash {
                let id = key_id(&item.key).to_owned();
                let Some(tracked) = self
                    .sessions
                    .with(cx.session_id(), |st| st.bash.get(&id).cloned())
                else {
                    continue;
                };
                let expect = if let Some(e) = tracked.expect {
                    e
                } else {
                    let answers = jev::ask(
                        &jev_settings,
                        &format!("Tool: bash\nCommand: {}", tracked.display),
                        &json!({"expect": prompt::expect_question()}),
                    )
                    .await;
                    let e = Expect::parse(
                        answers
                            .as_ref()
                            .and_then(|a| a.get("expect"))
                            .and_then(|a| a.choice.as_deref()),
                    );
                    self.sessions.with(cx.session_id(), |st| {
                        if let Some(t) = st.bash.get_mut(&id) {
                            t.expect = Some(e);
                        }
                    });
                    e
                };
                let pid = serde_json::from_str::<Value>(&control::running_bash())
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .and_then(|list| {
                        list.iter()
                            .find(|r| {
                                r.get("toolCallId").and_then(Value::as_str) == Some(id.as_str())
                            })
                            .and_then(|r| r.get("pid").and_then(Value::as_u64))
                    })
                    .and_then(|p| u32::try_from(p).ok());
                let sample = match pid {
                    Some(p) => Self::sample(p, &tracked.display).await,
                    None => None,
                };
                let Some(tracked) = self
                    .sessions
                    .with(cx.session_id(), |st| st.bash.get(&id).cloned())
                else {
                    continue;
                };
                let check_at = timers::now_ms();
                #[allow(clippy::cast_precision_loss, reason = "seconds")]
                let age = (check_at - tracked.started_at) as f64 / 1000.0;
                #[allow(clippy::cast_precision_loss, reason = "seconds")]
                let quiet = (check_at - tracked.last_change_at) as f64 / 1000.0;
                let previous = tracked.prev_age.map(|a| {
                    (
                        a,
                        tracked.last_change_at > tracked.prev_check_at.unwrap_or(i64::MAX),
                    )
                });
                let state = prompt::bash_state(
                    &tracked.display,
                    age,
                    quiet,
                    tracked.total_bytes,
                    &tracked.last_tail,
                    sample.as_ref(),
                    previous,
                );
                let answers = jev::ask(&jev_settings, &state, &prompt::bash_questions()).await;
                let stop = answers
                    .as_ref()
                    .and_then(|a| a.get("stop"))
                    .and_then(|a| a.noul);
                let bash = decide::decide_bash(&BashCheck {
                    age_sec: age,
                    since_output_sec: quiet,
                    command: &tracked.display,
                    sample: sample.as_ref(),
                    prev_idle: tracked.prev_idle,
                    prev_cumulative_io: tracked.prev_cumulative_io,
                    stop,
                    stop_streak: tracked.stop_streak,
                    expect,
                    threshold: s.confidence,
                    agree_checks: s.agree_checks,
                });
                self.sessions.with(cx.session_id(), |st| {
                    if let Some(t) = st.bash.get_mut(&id) {
                        t.prev_idle = bash.idle;
                        t.prev_cumulative_io = bash.cumulative_io;
                        t.stop_streak = bash.stop_streak;
                        t.prev_age = Some(age);
                        t.prev_check_at = Some(check_at);
                    }
                });
                bash_reason.clone_from(&bash.reason);
                self.debug(&format!(
                    "bash decision {}: act={} trigger={:?} reason={}",
                    item.key, bash.act, bash.trigger, bash.reason
                ));
                decide::Decision {
                    act: bash.act,
                    status: decide::Status::parse(
                        answers
                            .as_ref()
                            .and_then(|a| a.get("status"))
                            .and_then(|a| a.choice.as_deref()),
                    ),
                    confidence: stop.unwrap_or(0.0),
                    streak: bash.stop_streak,
                    signal: String::new(),
                    persistent: false,
                }
            } else {
                let state = prompt::generic_state(
                    &item.tool_name,
                    &item.command,
                    round_sec(now - item.started_at),
                    since_last_output,
                    output_changed,
                    &item.tail,
                );
                let answers = jev::ask(&jev_settings, &state, &prompt::generic_questions()).await;
                evaluate_tick(
                    answers.as_ref(),
                    previous_streak,
                    s.confidence,
                    s.agree_checks,
                )
            };
            self.sessions.with(cx.session_id(), |st| {
                st.streaks.insert(item.key.clone(), decision.streak);
                st.tails.insert(item.key.clone(), item.tail.clone());
                st.checked.insert(item.key.clone(), now);
            });
            self.debug(&format!(
                "tick {}: status={} confidence={:.2} streak={}/{} act={}",
                item.key,
                decision.status.as_str(),
                decision.confidence,
                decision.streak,
                s.agree_checks,
                decision.act
            ));
            if !decision.act {
                continue;
            }
            let reason = if bash_reason.is_empty() {
                format!(
                    "{} (confidence {:.2}) on {} consecutive checks — {}",
                    decision.status.as_str(),
                    decision.confidence,
                    decision.streak,
                    if output_changed {
                        "output keeps repeating without progress".to_owned()
                    } else {
                        format!("no new output for {since_last_output}s")
                    }
                )
            } else {
                bash_reason
            };
            self.act(cx, &s, &item, &decision, &reason, now - item.started_at)
                .await;
        }
    }

    async fn act(
        &self,
        cx: &Ctx,
        s: &Settings,
        item: &Item,
        decision: &decide::Decision,
        reason: &str,
        duration_ms: i64,
    ) {
        match item.kind {
            ItemKind::Bash => {
                let id = key_id(&item.key).to_owned();
                let detached = control::bash_detach(&id, reason);
                let handled = match (s.action, detached) {
                    (Action::Background, Ok(task)) => {
                        self.debug(&format!(
                            "backgrounded bash {} as {task} ({reason})",
                            item.command
                        ));
                        true
                    }
                    (Action::Kill, Ok(task)) => {
                        let _ = tasks::kill(&task);
                        self.sessions.with(cx.session_id(), |st| {
                            st.kills.insert(
                                id.clone(),
                                Kill {
                                    duration_ms,
                                    reason: format!("{}; {reason}", decision.signal),
                                },
                            );
                        });
                        self.debug(&format!("killed bash {} ({reason})", item.command));
                        true
                    }
                    (Action::Warn, Ok(_)) => true,
                    (_, Err(e)) => {
                        self.debug(&format!(
                            "bash {}: detach unavailable ({e}) — downgraded to warn",
                            item.command
                        ));
                        false
                    }
                };
                if handled && s.action != Action::Warn {
                    self.sessions.with(cx.session_id(), |st| {
                        st.bash.remove(&id);
                    });
                } else {
                    self.queue_warn(cx, item, decision, reason);
                }
            }
            ItemKind::Bg => {
                let id = key_id(&item.key);
                let running = tasks::get(id)
                    .ok()
                    .flatten()
                    .is_some_and(|t| t.get("state").and_then(Value::as_str) == Some("running"));
                if running {
                    match tasks::kill(id) {
                        Ok(()) => self.debug(&format!("killed bg task {id} ({reason})")),
                        Err(e) => self.debug(&format!("bg kill failed for {id}: {e}")),
                    }
                }
            }
            ItemKind::Other => match s.other_tools {
                OtherTools::AbortTurn if cx.has_ui() => {
                    let text = format!(
                        "⚠ Watchdog aborted \"{}\": jev judged it {reason}.",
                        item.tool_name
                    );
                    control::abort();
                    let opts = UserMessageOptions {
                        deliver_as: Some(session::Delivery::FollowUp),
                        expand_prompt_templates: false,
                        provenance: Some(Provenance {
                            source: "Watchdog".into(),
                            title: Some("Tool aborted".into()),
                            synopsis: Some("jev judged the tool stuck".into()),
                            severity: Some("warning".into()),
                        }),
                    };
                    let _ = oino_extension::wit::session::send_user_message(
                        json!(text).to_string(),
                        opts,
                    )
                    .await;
                }
                OtherTools::Warn => {
                    self.queue_warn(cx, item, decision, reason);
                    if cx.has_ui() {
                        let _ = ui::notify(
                            &format!(
                                "⚠ Watchdog: \"{}\" looks {}.",
                                item.tool_name,
                                decision.status.as_str()
                            ),
                            "warning",
                        );
                    }
                }
                _ => {}
            },
        }
    }

    fn queue_warn(&self, cx: &Ctx, item: &Item, decision: &decide::Decision, reason: &str) {
        let dedupe = format!("{}:{}", item.key, decision.status.as_str());
        self.sessions.with(cx.session_id(), |st| {
            if st.warned.insert(dedupe) {
                st.pending.push(warn_text(
                    &item.tool_name,
                    decision.status.as_str(),
                    decision.confidence,
                    decision.streak,
                    reason,
                ));
            }
        });
    }

    // ── commands ────────────────────────────────────────────────────────────

    fn bg_detach(args: &str) {
        let reason = if args.trim().is_empty() {
            "manual /unipi:bg-detach"
        } else {
            args.trim()
        };
        let latest = serde_json::from_str::<Value>(&control::running_bash())
            .ok()
            .and_then(|v| v.as_array().cloned())
            .and_then(|list| {
                list.into_iter()
                    .max_by_key(|r| r.get("startedAt").and_then(Value::as_i64).unwrap_or(0))
            })
            .and_then(|r| {
                r.get("toolCallId")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            });
        let detached = latest.and_then(|id| control::bash_detach(&id, reason).ok());
        let _ = match detached {
            Some(task) => {
                let output = tasks::get(&task)
                    .ok()
                    .flatten()
                    .and_then(|t| {
                        t.get("outputPath")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_default();
                ui::notify(
                    &format!(
                        "Moved the running bash call to background task {task} (output: {output})"
                    ),
                    "info",
                )
            }
            None => ui::notify(
                "Could not detach: no bash call is currently running, or background detachment is unavailable.",
                "warning",
            ),
        };
    }

    // ── session caps ────────────────────────────────────────────────────────

    fn caps(&self) -> Caps {
        Caps::resolve(&self.settings.get(NAMESPACE).0, get_flag)
    }

    async fn on_turn_end(&self, ev: &Event, cx: &Ctx) {
        let caps = self.caps();
        let now = timers::now_ms();
        let (verdict, run) = self.sessions.with(cx.session_id(), |st| {
            (st.run.turn_end(&caps, &ev.payload, now), st.run.clone())
        });
        let Verdict::Abort { reason, summary } = verdict else {
            return;
        };
        control::abort();
        let _ = ui::notify(&caps::notice(&reason), "warning");
        let _ = session::append_entry(
            "watchdog.session-cap",
            Some(
                &json!({"reason": reason, "turns": run.turns, "costUsd": run.cost_usd, "consecutiveToolErrors": run.consecutive_tool_errors}),
            ),
        );
        if summary {
            let _ = session::send_user_message(
                &json!(caps::summary_message(&reason)),
                Some(session::Delivery::FollowUp),
            )
            .await;
        }
    }
}

/// Last `n` non-empty lines (unipi bg `outputTail`).
fn unipi_last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[allow(clippy::cast_precision_loss, reason = "seconds")]
fn round_sec(ms: i64) -> i64 {
    js_round(ms as f64 / 1000.0)
}

impl Extension for Watchdog {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        for (name, description) in [
            ("max-turns", "Session cap: abort a run after N turns"),
            ("max-cost", "Session cap: abort a run after spending N USD"),
            ("max-wall", "Session cap: abort a run after N minutes"),
            (
                "max-tool-errors",
                "Session cap: abort after N consecutive tool errors",
            ),
        ] {
            cx.flag(Flag::string(name, description));
        }
        cx.command(
            Command::<Self>::new(unipi_kit::suite::command("bg-detach"))
                .description("Move the currently running bash call to a background task")
                .argument_hint("[reason]")
                .run(|_ext, args, _cx| async move {
                    Self::bg_detach(&args);
                    Ok(())
                }),
        );
        cx.on(EventKind::SessionStart, |ext, _ev, _cx| async move {
            ext.start_timer();
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            timers::cancel(HEARTBEAT);
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                ext.start_timer();
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            if ev.get("timerId").and_then(Value::as_str) == Some(HEARTBEAT) {
                ext.tick(&cx).await;
            }
            Ok(())
        });
        cx.on(EventKind::ToolExecutionStart, |ext, ev, cx| async move {
            ext.on_tool_start(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::ToolExecutionUpdate, |ext, ev, cx| async move {
            ext.on_tool_update(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::ToolExecutionEnd, |ext, ev, cx| async move {
            ext.on_tool_end(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::ToolResult, |ext, ev, cx| async move {
            Ok(ext.on_tool_result(&ev, &cx))
        });
        cx.on(EventKind::BeforeAgentStart, |ext, _ev, cx| async move {
            Ok(ext.on_before_agent_start(&cx))
        });
        cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
            let now = timers::now_ms();
            ext.sessions.with(cx.session_id(), |st| st.run.start(now));
            Ok(())
        });
        cx.on(EventKind::TurnEnd, |ext, ev, cx| async move {
            ext.on_turn_end(&ev, &cx).await;
            Ok(())
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Watchdog);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_match_unipi() {
        assert_eq!(
            kill_warning(125_400, "judged stuck; no output"),
            "⚠ Killed by unipi watchdog after 125s: jev judged it judged stuck; no output. Do not blindly re-run; investigate or change approach."
        );
        assert_eq!(
            warn_text("fetch", "stuck", 0.912, 2, "no new output for 300s"),
            "⚠ Watchdog: \"fetch\" looks stuck (confidence 0.91, 2 consecutive checks): no new output for 300s."
        );
        assert_eq!(key_id("bash:abc:1"), "abc:1");
        assert_eq!(unipi_last_lines("a\n\nb\nc\nd\n", 3), "b\nc\nd");
        assert!(NEVER_WATCH.contains(&"read_subagent"));
    }
}
