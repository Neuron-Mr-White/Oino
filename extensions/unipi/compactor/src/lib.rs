//! `unipi.compactor` — lossless compaction and recall (unipi `packages/compactor`,
//! unipi-port §3.15).
//!
//! | unipi | here |
//! |---|---|
//! | every compaction through `session_before_compact` → `vcc` (zero-LLM) or `llm` | [`Compactor::before_compact`]; `llm` uses host `models.summarize-default` |
//! | vcc: full raw branch, origin marks, redaction, fixed section shares, recall note | [`source`], [`summary`], [`brief`], [`cut`], [`plan`] |
//! | percent trigger: compaction draft at `turn_end`, cooldown 60 s, growth 4 000 | [`config::decide`] + `turn_end` boundary drafts |
//! | `/unipi:compact-then <prompt>` via `input` (+ autocomplete entry) | `input` handler + `complete-input` |
//! | compaction card `unipi-compaction` + renderer, `COMPACTOR_COMPACTED` | custom entry + OUS renderer, bus topic |
//! | active work (`registerCompactionContext`) | fan-out `compaction-context.provide` |
//! | `session_recall`, `context_budget` | tools ([`recall`], [`card::context_budget`]) |
//! | `/unipi:compact-vcc`, `-by-llm`, `session-recall`, `compact-stats`, `-doctor`, `-help` | commands |
//! | info-screen "Compactor" group | service `info-screen.groups` |
//! | `compactor` settings | [`config`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod blocks;
pub mod brief;
pub mod card;
pub mod config;
pub mod cut;
pub mod plan;
pub(crate) mod re;
pub mod recall;
pub mod source;
pub mod summary;

use std::cell::Cell;

use oino_extension::prelude::*;
use oino_extension::session::{CustomMessage, Delivery, Provenance, SendOptions};
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use oino_extension::wit::session as ws;
use unipi_kit::services::fanout as fan;
use unipi_kit::settings::Store;
use unipi_kit::topics::{Compacted, ModuleReady};

use crate::brief::FileOps;
use crate::card::{CARD_TYPE, COMPACTOR_ID, CardInput};
use crate::config::{AutoState, Config, EXTENSION_ID, Method, NAMESPACE};
use crate::cut::{COMPACTOR_INSTRUCTION, apply_tail_budget, build_own_cut, parse_instructions};
use crate::plan::{PlanInput, Stats, plan_lossless};

const LEGACY_HIDDEN_TYPES: &[&str] = &["compactor-auto-continue", "unipi-compactor-resume"];
const RECALL_TYPE: &str = "compactor-recall";

/// `/unipi:compact-then <prompt>` match → prompt.
#[must_use]
pub fn compact_then_prompt(text: &str) -> Option<String> {
    let rest = text.strip_prefix("/unipi:compact-then")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim().to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueMode {
    Steer,
    FollowUp,
}

#[derive(Default)]
struct Sess {
    pending_method: Option<Method>,
    command_compaction: bool,
    pending_command: Option<String>,
    last_command: Option<String>,
    last_reason: Option<String>,
    last_stats: Option<Stats>,
    last_sections: Option<Vec<String>>,
    pending_follow_up: Option<String>,
    compact_then: Option<(String, QueueMode)>,
    auto: AutoState,
}

/// The extension.
#[derive(Default)]
pub struct Compactor {
    settings: Store,
    sessions: SessionMap<Sess>,
    recall_queries: Cell<u64>,
    compactions: Cell<u64>,
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

fn provenance(title: &str, synopsis: &str) -> Provenance {
    Provenance {
        source: "Compactor".into(),
        title: Some(title.into()),
        synopsis: Some(synopsis.into()),
        severity: None,
    }
}

/// Sends `text` as a harness user message.
async fn send_user(text: &str, title: &str, deliver: Delivery) {
    let _ = ws::send_user_message(
        json!(text).to_string(),
        ws::UserMessageOptions {
            deliver_as: Some(deliver),
            expand_prompt_templates: false,
            provenance: Some(provenance(title, "Prompt queued after compaction")),
        },
    )
    .await;
}

fn usage() -> (Option<f64>, Option<f64>, Option<f64>) {
    let u = control::context_usage().unwrap_or(Value::Null);
    (
        u.get("tokens").and_then(Value::as_f64),
        u.get("percent").and_then(Value::as_f64),
        u.get("contextWindow").and_then(Value::as_f64),
    )
}

fn context_window() -> Option<u64> {
    control::model()
        .and_then(|m| m.get("contextWindow").and_then(Value::as_u64))
        .or_else(|| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "window"
            )]
            usage().2.map(|w| w as u64)
        })
}

fn ops_of(prep: Option<&Value>) -> FileOps {
    let list = |k: &str| -> Vec<String> {
        prep.and_then(|p| p.pointer(&format!("/fileOps/{k}")))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    };
    let mut modified = list("written");
    modified.extend(list("edited"));
    FileOps {
        read: list("read").into_iter().collect(),
        modified: modified.into_iter().collect(),
    }
}

fn join_instructions(parts: &[Option<&str>]) -> Option<String> {
    let t = parts
        .iter()
        .flatten()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!t.is_empty()).then_some(t)
}

fn emit_compacted(stats: Option<&Stats>, method: Method) {
    let before = stats.and_then(|s| s.tokens_before).unwrap_or(0);
    let after = stats.map_or(0, |s| s.tokens_after);
    unipi_kit::bus::emit(&Compacted {
        method: method.as_str().into(),
        summarized: stats.map_or(0, |s| s.summarized as u64),
        kept: stats.map_or(0, |s| s.kept as u64),
        tokens_before: before,
        tokens_after: after,
        tokens_saved: before.saturating_sub(after),
    });
}

fn card_input(
    method: Method,
    trigger: &'static str,
    command: Option<String>,
    tokens_before: u64,
    stats: Option<&Stats>,
    sections: Option<Vec<String>>,
    summary: &str,
) -> CardInput {
    CardInput {
        method: method.as_str(),
        trigger,
        command,
        tokens_before,
        tokens_after: stats.map(|s| s.tokens_after),
        summary_chars: summary.chars().count(),
        kept: stats.map(|s| (s.kept_user_turns, s.total_user_turns, s.kept_tokens)),
        sections,
        percent: None,
        threshold: None,
    }
}

impl Compactor {
    fn cfg(&self) -> Config {
        config::load(&self.settings.get(NAMESPACE))
    }

    async fn active_work() -> Vec<(String, String)> {
        let replies = unipi_kit::fanout::call_all(
            fan::COMPACTION_CONTEXT_PROVIDE,
            &json!({}),
            unipi_kit::fanout::DEFAULT_TIMEOUT,
            Some(EXTENSION_ID),
        )
        .await;
        let mut out = Vec::new();
        for r in replies {
            let items = match r.value {
                Value::Array(a) => a,
                v @ Value::Object(_) => vec![v],
                _ => Vec::new(),
            };
            for v in items {
                if let (Some(id), Some(text)) = (
                    v.get("id").and_then(Value::as_str),
                    v.get("text")
                        .and_then(Value::as_str)
                        .filter(|t| !t.trim().is_empty()),
                ) {
                    out.push((id.to_owned(), text.to_owned()));
                }
            }
        }
        out
    }

    fn debug(&self, data: &Value) {
        if self.cfg().debug {
            let _ = std::fs::write(
                unipi_kit::paths::logs("compactor-debug.json"),
                serde_json::to_vec_pretty(data).unwrap_or_default(),
            );
        }
    }

    async fn with_active_work(summary: String) -> String {
        let blocks = Self::active_work().await;
        if blocks.is_empty() {
            return summary;
        }
        let text = blocks
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        format!("[Active Work]\n{text}\n\n{summary}")
    }

    /// `session_before_compact`.
    #[allow(clippy::too_many_lines, reason = "two methods of one hook")]
    async fn before_compact(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let cfg = self.cfg();
        let sid = cx.session_id().to_owned();
        let reason = ev
            .get("reason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let custom = ev.get("customInstructions").and_then(Value::as_str);
        let parsed = parse_instructions(custom);
        let prep = ev.get("preparation");
        let (pending, from_command, command) = self.sessions.with(&sid, |s| {
            let p = s.pending_method.take();
            let fc = p.is_some() && s.command_compaction;
            let c = s.pending_command.take();
            s.last_reason.clone_from(&reason);
            s.last_command = if fc { c.clone() } else { None };
            s.last_sections = None;
            s.pending_follow_up = None;
            (p, fc, c)
        });
        let _ = command;
        let method = pending.unwrap_or(if parsed.is_compactor {
            Method::Vcc
        } else if reason.as_deref() == Some("manual") && cfg.pi_compact.is_some() {
            cfg.pi_compact.unwrap_or(cfg.method)
        } else {
            cfg.method
        });
        let tokens_before = prep
            .and_then(|p| p.get("tokensBefore"))
            .and_then(Value::as_u64);
        if method == Method::Llm {
            let user = if parsed.is_compactor { None } else { custom };
            let mut messages: Vec<Value> = prep
                .and_then(|p| p.get("messagesToSummarize"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages.extend(
                prep.and_then(|p| p.get("turnPrefixMessages"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            let instructions = join_instructions(&[user, Some(cfg.llm_instructions.as_str())]);
            return match models::summarize_default(&Value::Array(messages), instructions.as_deref())
                .await
            {
                Ok(r) => {
                    let summary = Self::with_active_work(
                        r.get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    )
                    .await;
                    self.sessions.with(&sid, |s| s.last_stats = None);
                    let mut c = json!({
                        "summary": summary,
                        "firstKeptEntryId": prep.and_then(|p| p.get("firstKeptEntryId")).cloned().unwrap_or(json!("")),
                        "tokensBefore": tokens_before.unwrap_or(0),
                        "details": {"compactor": COMPACTOR_ID, "version": 2, "method": "llm"},
                    });
                    if let Some(u) = r.get("usage").filter(|u| !u.is_null()) {
                        c["usage"] = u.clone();
                    }
                    Some(json!({"compaction": c}))
                }
                Err(e) => {
                    self.debug(&json!({"method": "llm", "fallback": true, "error": e.to_string()}));
                    None
                }
            };
        }
        let branch: Vec<Value> = ev
            .get("branchEntries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let active = Self::active_work().await;
        let cwd = cx.cwd().display().to_string();
        let plan = plan_lossless(&PlanInput {
            branch: &branch,
            tokens_before,
            previous_summary: prep
                .and_then(|p| p.get("previousSummary"))
                .and_then(Value::as_str),
            ops: ops_of(prep),
            keep: Some(parsed.keep),
            explicit: parsed.explicit,
            context_window: context_window(),
            config: &cfg,
            cwd: Some(&cwd),
            reason: reason.as_deref(),
            active_work: &active,
        });
        let plan = match plan {
            Ok(p) => p,
            Err(refusal) => {
                if reason.as_deref() == Some("overflow") {
                    return None;
                }
                if (reason.as_deref() == Some("manual") || from_command) && cx.has_ui() {
                    let _ = ui::notify(
                        if refusal == cut::Refusal::NoLiveMessages {
                            "compactor: nothing to compact"
                        } else {
                            "compactor: too few messages to compact"
                        },
                        "warning",
                    );
                }
                self.debug(&json!({"cancelled": true, "reason": refusal.as_str()}));
                return Some(json!({"cancel": true}));
            }
        };
        self.sessions.with(&sid, |s| {
            s.last_stats = Some(plan.stats.clone());
            s.last_sections = Some(plan.sections.clone());
            if !parsed.is_compactor && reason.as_deref() == Some("manual") {
                s.pending_follow_up.clone_from(&parsed.follow_up);
            }
        });
        self.debug(&json!({"method": "vcc", "reason": reason, "details": plan.details, "summaryPreview": plan.summary.chars().take(800).collect::<String>()}));
        Some(json!({"compaction": {
            "summary": plan.summary,
            "details": plan.details,
            "tokensBefore": tokens_before.unwrap_or(0),
            "firstKeptEntryId": plan.first_kept,
        }}))
    }

    /// `session_compact`.
    async fn on_compact(&self, ev: &Event, cx: &Ctx) {
        let entry = ev.get("compactionEntry").cloned().unwrap_or(Value::Null);
        let details = entry.get("details");
        let ours = details
            .and_then(|d| d.get("compactor"))
            .and_then(Value::as_str)
            == Some(COMPACTOR_ID);
        let method = if ours
            && details
                .and_then(|d| d.get("method"))
                .and_then(Value::as_str)
                != Some("llm")
        {
            Method::Vcc
        } else {
            Method::Llm
        };
        let (stats, sections, was_command, last_command, reason, follow) =
            self.sessions.with(cx.session_id(), |s| {
                let wc = s.command_compaction;
                s.command_compaction = false;
                (
                    s.last_stats.clone(),
                    s.last_sections.clone(),
                    wc,
                    s.last_command.clone(),
                    s.last_reason.clone(),
                    s.pending_follow_up.take(),
                )
            });
        self.compactions.set(self.compactions.get() + 1);
        emit_compacted(if ours { stats.as_ref() } else { None }, method);
        if was_command || self.cfg().notify {
            let trigger = match reason.as_deref() {
                Some("threshold") => "threshold",
                Some("overflow") => "overflow",
                _ => "manual",
            };
            let before = entry
                .get("tokensBefore")
                .and_then(Value::as_u64)
                .or_else(|| stats.as_ref().and_then(|s| s.tokens_before))
                .unwrap_or(0);
            let data = card::card_data(&card_input(
                method,
                trigger,
                if was_command { last_command } else { None },
                before,
                if ours { stats.as_ref() } else { None },
                if ours { sections } else { None },
                entry
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ));
            let _ = session::append_entry(CARD_TYPE, Some(&data));
        }
        if let Some(f) = follow {
            send_user(&f, "Follow-up", Delivery::FollowUp).await;
        }
    }

    fn compact_now(&'static self, cx: &Ctx, prompt: String, command: &str) {
        let sid = cx.session_id().to_owned();
        self.sessions.with(&sid, |s| {
            s.pending_method = None;
            s.command_compaction = true;
            s.pending_command = Some(command.to_owned());
        });
        let has_ui = cx.has_ui();
        spawn(async move {
            match control::compact(Some(COMPACTOR_INSTRUCTION)).await {
                Ok(_) => {
                    if !prompt.is_empty() {
                        send_user(&prompt, "Compact then", Delivery::FollowUp).await;
                    }
                }
                Err(e) => {
                    if has_ui {
                        let _ = ui::notify(&format!("Compaction failed: {e}"), "error");
                    }
                }
            }
        });
    }

    fn on_input(&'static self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let text = ev
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if ev.get("source").and_then(Value::as_str) == Some("extension") && !text.trim().is_empty()
        {
            let _ = session::append_entry(
                source::ORIGIN_ENTRY_TYPE,
                Some(&json!({"key": source::origin_key(&text)})),
            );
        }
        let prompt = compact_then_prompt(&text)?;
        let streaming = ev.get("streamingBehavior").and_then(Value::as_str);
        match streaming {
            None => self.compact_now(cx, prompt, "unipi:compact-then"),
            Some(mode) => {
                let mode = if mode == "followUp" {
                    QueueMode::FollowUp
                } else {
                    QueueMode::Steer
                };
                self.sessions
                    .with(cx.session_id(), |s| s.compact_then = Some((prompt, mode)));
                if cx.has_ui() {
                    let _ = ui::notify(
                        if mode == QueueMode::FollowUp {
                            "Compact then: queued — will compact and continue after this run."
                        } else {
                            "Compact then: queued — will compact and continue at the next turn."
                        },
                        "info",
                    );
                }
            }
        }
        Some(json!({"action": "handled"}))
    }

    fn on_settled(&'static self, cx: &Ctx) {
        let queued = self.sessions.with(cx.session_id(), |s| {
            if s.compact_then
                .as_ref()
                .is_some_and(|(_, m)| *m == QueueMode::FollowUp)
            {
                s.compact_then.take()
            } else {
                None
            }
        });
        if let Some((prompt, _)) = queued {
            self.compact_now(cx, prompt, "unipi:compact-then");
        }
    }

    async fn boundary_draft(
        &self,
        cfg: &Config,
        branch: &[Value],
        tokens: Option<u64>,
        cwd: &str,
    ) -> Option<(Value, Option<Stats>, Option<Vec<String>>)> {
        if cfg.method == Method::Llm {
            let cut = build_own_cut(branch, 1).ok()?;
            let tail = apply_tail_budget(branch, cut, None, None, cut::DEFAULT_CPT);
            if tail.messages.is_empty() {
                return None;
            }
            let previous = branch
                .iter()
                .rev()
                .find(|e| e.get("type").and_then(Value::as_str) == Some("compaction"))
                .and_then(|e| e.get("summary").and_then(Value::as_str))
                .map(|p| format!("Update this previous summary with the new messages:\n{p}"));
            let instructions =
                join_instructions(&[Some(cfg.llm_instructions.as_str()), previous.as_deref()]);
            let r = models::summarize_default(
                &Value::Array(tail.messages.clone()),
                instructions.as_deref(),
            )
            .await
            .ok()?;
            let summary = Self::with_active_work(
                r.get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
            .await;
            let mut entry = json!({"type": "compaction", "summary": summary, "firstKeptEntryId": if tail.first_kept.is_empty() { Value::Null } else { json!(tail.first_kept) },
                "details": {"compactor": COMPACTOR_ID, "version": 2, "method": "llm", "reason": "percent"}});
            if let Some(u) = r.get("usage").filter(|u| !u.is_null()) {
                entry["usage"] = u.clone();
            }
            return Some((entry, None, None));
        }
        let active = Self::active_work().await;
        let plan = plan_lossless(&PlanInput {
            branch,
            tokens_before: tokens,
            previous_summary: None,
            ops: FileOps::default(),
            keep: None,
            explicit: false,
            context_window: context_window(),
            config: cfg,
            cwd: Some(cwd),
            reason: Some("percent"),
            active_work: &active,
        })
        .ok()?;
        let entry = json!({"type": "compaction", "summary": plan.summary, "firstKeptEntryId": if plan.first_kept.is_empty() { Value::Null } else { json!(plan.first_kept) }, "details": plan.details});
        Some((entry, Some(plan.stats), Some(plan.sections)))
    }

    /// `turn_end`: compact-then (steer) first, then the percentage trigger.
    #[allow(clippy::too_many_lines, reason = "two boundary paths of one hook")]
    async fn on_turn_end(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let cfg = self.cfg();
        let cwd = cx.cwd().display().to_string();
        let sid = cx.session_id().to_owned();
        let original: Vec<Value> = ev
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut entries = original.clone();
        let aborted = ev.get("outcome").and_then(Value::as_str) == Some("aborted");
        let has_compaction = |es: &[Value]| {
            es.iter()
                .any(|d| d.get("type").and_then(Value::as_str) == Some("compaction"))
        };
        let queued = self.sessions.with(&sid, |s| {
            if s.compact_then
                .as_ref()
                .is_some_and(|(_, m)| *m == QueueMode::Steer)
            {
                s.compact_then.take()
            } else {
                None
            }
        });
        if let Some((prompt, _)) = queued
            && !aborted
        {
            if !has_compaction(&entries) {
                let branch = session::branch(None).unwrap_or_default();
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "tokens"
                )]
                let tokens = usage().0.map(|t| t as u64);
                match self.boundary_draft(&cfg, &branch, tokens, &cwd).await {
                    Some((entry, stats, sections)) => {
                        emit_compacted(stats.as_ref(), cfg.method);
                        self.compactions.set(self.compactions.get() + 1);
                        if cfg.notify {
                            let before = entry
                                .pointer("/usage/tokensBefore")
                                .and_then(Value::as_u64)
                                .or_else(|| stats.as_ref().and_then(|s| s.tokens_before))
                                .unwrap_or(0);
                            let data = card::card_data(&card_input(
                                cfg.method,
                                "manual",
                                Some("unipi:compact-then".into()),
                                before,
                                stats.as_ref(),
                                sections,
                                entry
                                    .get("summary")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                            ));
                            entries.push(entry);
                            entries.push(
                                json!({"type": "custom", "customType": CARD_TYPE, "data": data}),
                            );
                        } else {
                            entries.push(entry);
                        }
                    }
                    None => {
                        if cfg.notify && cx.has_ui() {
                            let _ = ui::notify(
                                "compactor: nothing to compact for /unipi:compact-then",
                                "warning",
                            );
                        }
                    }
                }
            }
            if !prompt.is_empty() {
                send_user(&prompt, "Compact then", Delivery::Steer).await;
            }
        }
        if cfg.percent && !aborted && !has_compaction(&entries) {
            let (tokens, percent, _) = usage();
            let st = self.sessions.with(&sid, |s| s.auto);
            let d = config::decide(
                true,
                cfg.threshold,
                tokens.zip(percent),
                st,
                timers::now_ms(),
            );
            self.sessions.with(&sid, |s| s.auto = d.state);
            if d.trigger {
                let branch = session::branch(None).unwrap_or_default();
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "tokens"
                )]
                let t = d.usage.map(|u| u.0 as u64);
                match self.boundary_draft(&cfg, &branch, t, &cwd).await {
                    None => self.sessions.with(&sid, |s| {
                        s.auto = config::mark_error(s.auto, timers::now_ms());
                    }),
                    Some((entry, stats, sections)) => {
                        self.sessions
                            .with(&sid, |s| s.auto = config::mark_complete(s.auto));
                        self.compactions.set(self.compactions.get() + 1);
                        emit_compacted(stats.as_ref(), cfg.method);
                        let card = cfg.notify.then(|| {
                            let before = entry.pointer("/usage/tokensBefore").and_then(Value::as_u64).or(t).unwrap_or(0);
                            let mut ci = card_input(cfg.method, "percent", None, before, stats.as_ref(), sections, entry.get("summary").and_then(Value::as_str).unwrap_or_default());
                            ci.percent = d.usage.map(|u| u.1);
                            ci.threshold = Some(cfg.threshold);
                            json!({"type": "custom", "customType": CARD_TYPE, "data": card::card_data(&ci)})
                        });
                        entries.push(entry);
                        entries.extend(card);
                    }
                }
            }
        }
        (entries != original).then(|| json!({"entries": entries}))
    }

    fn recall_blocks(all: bool) -> Vec<blocks::Block> {
        let entries = if all {
            session::entries()
        } else {
            session::branch(None)
        };
        blocks::filter_noise(blocks::recall_blocks(&entries.unwrap_or_default()))
    }

    async fn run_compaction(&self, cx: &CommandCtx, method: Method, args: &str, command: &str) {
        let trimmed = args.trim();
        self.sessions.with(cx.session_id(), |s| {
            s.pending_method = Some(method);
            s.command_compaction = true;
            s.pending_command = Some(command.to_owned());
        });
        let instructions = match method {
            Method::Llm => (!trimmed.is_empty()).then(|| trimmed.to_owned()),
            Method::Vcc => Some(if trimmed.is_empty() {
                COMPACTOR_INSTRUCTION.to_owned()
            } else {
                format!("{COMPACTOR_INSTRUCTION} {trimmed}")
            }),
        };
        let result = control::compact(instructions.as_deref()).await;
        self.sessions.with(cx.session_id(), |s| {
            s.pending_method = None;
            s.pending_command = None;
        });
        if let Err(e) = result {
            let m = e.to_string();
            if m.to_lowercase().contains("too small") || !cx.has_ui() {
                return;
            }
            if m == "Compaction cancelled" || m == "Already compacted" {
                let _ = ui::notify("Nothing to compact.", "info");
            } else {
                let _ = ui::notify(&format!("Compaction failed: {m}"), "error");
            }
        }
    }

    async fn session_recall_command(&self, args: &str, cx: &CommandCtx) {
        let usage_text = "Usage: /unipi:session-recall <query>";
        let say = |m: &str| {
            if cx.has_ui() {
                let _ = ui::notify(m, "warning");
            }
        };
        let raw = args.trim();
        let (all, page, query) = recall::parse_command_args(raw);
        if raw.is_empty() || query.is_empty() {
            return say(usage_text);
        }
        let blocks = Self::recall_blocks(all);
        if blocks.is_empty() {
            return say("No session history available for search.");
        }
        self.recall_queries.set(self.recall_queries.get() + 1);
        let text = recall::recall(
            &blocks,
            &recall::Query {
                query: Some(query),
                all,
                page: Some(page),
                ..recall::Query::default()
            },
            &cx.cx.cwd().display().to_string(),
            &timers::now_ms,
        );
        let idle = control::is_idle();
        let _ = session::send_message(
            &CustomMessage {
                custom_type: RECALL_TYPE.into(),
                content: json!(text),
                display: true,
                details: None,
            },
            SendOptions {
                trigger_turn: true,
                deliver_as: (!idle).then_some(Delivery::FollowUp),
                provenance: Some(provenance(
                    "Session recall",
                    "Results from /unipi:session-recall",
                )),
            },
        )
        .await;
    }

    fn stats_text() -> String {
        use unipi_kit::fmt::format_tokens;
        let branch = session::branch(None).unwrap_or_default();
        let (rec, before, after, saved) = card::session_stats(&branch);
        let (tokens, _, window) = usage();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "display"
        )]
        let f = |x: f64| format_tokens(x.max(0.0) as u64);
        let mut lines = vec![
            "Compactor — this session".to_owned(),
            format!(
                "Compactions: {}{}",
                rec.len(),
                if rec.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", card::method_breakdown(&rec))
                }
            ),
            format!(
                "Tokens: {}",
                if before > 0.0 {
                    format!("{} → {} (−{})", f(before), f(after), f(saved))
                } else {
                    "nothing compacted yet".into()
                }
            ),
        ];
        if let (Some(t), Some(w)) = (tokens, window.filter(|w| *w > 0.0)) {
            lines.push(format!("Context now: ~{} / {}", f(t), f(w)));
        }
        lines.join("\n")
    }

    fn info_group(&self) -> Value {
        use unipi_kit::fmt::format_tokens;
        let cfg = self.cfg();
        let branch = session::branch(None).unwrap_or_default();
        let (rec, before, after, saved) = card::session_stats(&branch);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "display"
        )]
        let f = |x: f64| format_tokens(x.max(0.0) as u64);
        let when = if cfg.percent {
            format!("at {}% of context", cfg.threshold)
        } else {
            "at Oino's context limit".into()
        };
        let pc = cfg.pi_compact.map_or("same as method", Method::name);
        let pct = if before > 0.0 {
            (saved / before * 100.0).round()
        } else {
            0.0
        };
        let last = rec.last();
        let (items, totals) = unipi_kit::savings::compaction_savings(&branch);
        let now = timers::now_ms();
        let ago = |ms: i64| {
            let s = (ms.max(0) + 500) / 1000;
            if s < 60 {
                format!("{s}s ago")
            } else if s < 3600 {
                format!("{}m ago", (s + 30) / 60)
            } else if s < 86_400 {
                format!("{}h ago", (s + 1800) / 3600)
            } else {
                format!("{}d ago", (s + 43_200) / 86_400)
            }
        };
        json!({
            "id": "compactor", "name": "Compactor", "icon": "🗜️", "priority": 12,
            "config": {"showByDefault": true, "stats": [
                {"id": "settings", "label": "Method", "show": true},
                {"id": "compactions", "label": "Compactions", "show": true},
                {"id": "tokens", "label": "Tokens", "show": true},
                {"id": "last", "label": "Last", "show": true},
            ]},
            "data": {
                "settings": {"value": cfg.method.name(), "detail": format!("when: {} · /compact: {pc}", if cfg.percent { format!("{}% of context", cfg.threshold) } else { "Oino's context limit".into() })},
                "compactions": {"value": rec.len().to_string(), "detail": if rec.is_empty() { "none this session".into() } else { card::method_breakdown(&rec) }},
                "tokens": {"value": if rec.is_empty() { "—".into() } else { format!("{} → {}", f(before), f(after)) }, "detail": if rec.is_empty() { "nothing compacted yet".into() } else { format!("−{} ({pct}%) across all compactions", f(saved)) }},
                "last": {"value": last.map_or_else(|| "—".into(), |l| format!("{} → {}", f(l.before), f(l.after))), "detail": last.map_or_else(|| "no compaction yet".into(), |l| [Some(if l.method == Method::Vcc { "lossless" } else { "model summary" }.to_owned()), l.at.map(|a| ago(now - a))].into_iter().flatten().collect::<Vec<_>>().join(" · "))},
                "raw": {"value": "", "detail": "", "raw": {
                    "method": cfg.method.name(), "trigger": when, "piCompact": pc,
                    "history": rec.iter().rev().take(24).rev().map(|r| json!({"before": r.before, "after": r.after, "method": if r.method == Method::Vcc { "lossless" } else { "model summary" }, "at": r.at})).collect::<Vec<_>>(),
                    "before": before, "after": after, "saved": saved,
                    "session": totals, "items": items.iter().rev().take(24).rev().collect::<Vec<_>>(),
                    "recallQueries": self.recall_queries.get(), "compactions": self.compactions.get(),
                }},
            },
        })
    }
}

impl Extension for Compactor {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.settings.declare(NAMESPACE, config::defaults());
        self.settings.load(cx.settings());
        cx.settings_namespace(&config::namespace().to_json());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.settings.refresh();
            ext.sessions.remove(cx.session_id());
            unipi_kit::bus::emit(&ModuleReady {
                name: "@pi-unipi/compactor".into(),
                version: host::info().extension_version,
                commands: [
                    "compact-vcc",
                    "compact-by-llm",
                    "session-recall",
                    "compact-stats",
                    "compact-doctor",
                    "compact-help",
                ]
                .map(ToOwned::to_owned)
                .to_vec(),
                tools: vec!["session_recall".into(), "context_budget".into()],
                load_time_ms: None,
            });
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            let _ = ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        cx.on(EventKind::Context, |_ext, ev, _cx| async move {
            let messages: Vec<Value> = ev
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let n = messages.len();
            let kept: Vec<Value> = messages
                .into_iter()
                .filter(|m| {
                    m.get("role").and_then(Value::as_str) != Some("custom")
                        || !LEGACY_HIDDEN_TYPES.contains(
                            &m.get("customType")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                })
                .collect();
            Ok((kept.len() != n).then(|| json!({"messages": kept})))
        });
        cx.on(EventKind::Input, |ext, ev, cx| async move {
            Ok(ext.on_input(&ev, &cx))
        });
        cx.on(EventKind::SessionBeforeCompact, |ext, ev, cx| async move {
            Ok(ext.before_compact(&ev, &cx).await)
        });
        cx.on(EventKind::SessionCompact, |ext, ev, cx| async move {
            ext.on_compact(&ev, &cx).await;
            Ok(())
        });
        cx.on(EventKind::AgentEnd, |ext, _ev, cx| async move {
            ext.on_settled(&cx);
            Ok(())
        });
        cx.on(EventKind::AgentSettled, |ext, _ev, cx| async move {
            ext.on_settled(&cx);
            Ok(())
        });
        cx.on(EventKind::TurnEnd, |ext, ev, cx| async move {
            Ok(ext.on_turn_end(&ev, &cx).await)
        });
        cx.renderer(RendererTarget::CustomEntry, CARD_TYPE, |_ext, req, _cx| {
            let entry = req.get("data")?;
            let d = entry.get("data").unwrap_or(entry);
            Some(card::render(
                d,
                req.get("expanded")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ))
        });
        cx.input_completer(|_ext, req, _cx| async move {
            let text = req.get("text").and_then(Value::as_str).unwrap_or_default();
            #[allow(clippy::cast_possible_truncation, reason = "cursor positions are small")]
            let cursor = req.get("cursor").and_then(Value::as_u64).map_or(text.len(), |c| c as usize);
            let before: String = text.chars().take(cursor).collect();
            if !before.starts_with('/') || before.contains(' ') {
                return None;
            }
            let q = before[1..].to_lowercase();
            if !"unipi:compact-then".contains(&q) && !"compact-then".starts_with(&q) {
                return None;
            }
            Some(json!([{"source": EXTENSION_ID, "items": [{"value": "unipi:compact-then", "label": "unipi:compact-then", "description": "Compacts, then sends a prompt once it lands (not a registered command)"}]}]))
        });
        cx.service(
            fan::INFO_SCREEN_GROUPS,
            |ext, _p, _caller, _cx| async move { Ok(ext.info_group()) },
        );

        cx.tool(
            Tool::<Self>::new("session_recall")
                .label("Session Recall")
                .description("Search session history using keyword or regex search. Find previous goals, files, commits, decisions, and context — including anything dropped by compaction. Reach for this before telling the user you no longer have the context. Plain keywords work best; a regex pattern is also accepted. Results are paged (page); pass expand with entry indices to read full untruncated content. Use mode:'touched' to list files worked on in this session with their entry indices, and #N:path to drill into a file's content from an entry (#N:path:full for all lines). Only the current session is searchable — earlier sessions are not.")
                .parameters(&json!({"type": "object", "properties": {
                    "query": {"type": "string", "description": "What to recall, in plain keywords (e.g. 'redis cache decision'). Multi-word queries are ranked by relevance. A regex pattern also works. #N:path drills into a file's content from an entry."},
                    "expand": {"type": "array", "items": {"type": "number"}, "description": "Entry indices to return full untruncated content for"},
                    "page": {"type": "number", "minimum": 1, "description": "Page number (1-based) for paginated search results. Default: 1."},
                    "scope": {"anyOf": [{"const": "lineage", "type": "string"}, {"const": "all", "type": "string"}], "description": "Default 'lineage' covers the active conversation path. Use 'all' to also reach messages from other branches, such as turns that were edited or retried."},
                    "mode": {"anyOf": [{"const": "hybrid", "type": "string"}, {"const": "touched", "type": "string"}], "description": "What to show. hybrid (default) = normal search; touched = aggregated files-by-path with entry indices."},
                }}))
                .kind(ToolKind::Search)
                .run(|ext, args: Value, cx| async move {
                    ext.recall_queries.set(ext.recall_queries.get() + 1);
                    let all = args.get("scope").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("all"));
                    let q = recall::Query {
                        query: args.get("query").and_then(Value::as_str).map(ToOwned::to_owned),
                        all,
                        touched: args.get("mode").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("touched")),
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "page numbers")]
                        page: args.get("page").and_then(Value::as_f64).map(|p| p.max(1.0) as usize),
                        #[allow(clippy::cast_possible_truncation, reason = "indices")]
                        expand: args.get("expand").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_f64).map(|f| if f.fract() == 0.0 { f as i64 } else { -1 }).collect(),
                    };
                    let text = recall::recall(&Self::recall_blocks(all), &q, &cx.cx.cwd().display().to_string(), &timers::now_ms);
                    Ok(ToolResult::text(text).with_details(json!({"query": args.get("query").cloned().unwrap_or(Value::Null)})))
                }),
        );
        cx.tool(
            Tool::<Self>::new("context_budget")
                .label("Context Budget")
                .description("Estimate how full the context window is (% full, tokens left). Compaction is automatic; this never needs to be acted on.")
                .parameters(&json!({"type": "object", "properties": {}}))
                .kind(ToolKind::Think)
                .run(|ext, _args: Value, _cx| async move {
                    let (t, _, w) = usage();
                    Ok(ToolResult::text(card::context_budget(t, w, &ext.cfg())))
                }),
        );

        cx.command(
            Command::<Self>::new("unipi:compact-vcc")
                .description("Lossless compaction now — instant structured summary, no model call (keep:N keeps N recent turns)")
                .run(|ext, args, cx| async move {
                    ext.run_compaction(&cx, Method::Vcc, &args, "unipi:compact-vcc").await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:compact-by-llm")
                .description(
                    "Compact now with a model-written summary (optional text focuses the summary)",
                )
                .run(|ext, args, cx| async move {
                    ext.run_compaction(&cx, Method::Llm, &args, "unipi:compact-by-llm")
                        .await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:session-recall")
                .description("Recall earlier parts of this session. Plain keywords work best; add scope:all to reach edited or retried turns.")
                .run(|ext, args, cx| async move {
                    ext.session_recall_command(&args, &cx).await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:compact-stats")
                .description("Show this session's compaction savings")
                .run(|_ext, _args, cx| async move {
                    if cx.has_ui() {
                        ui::notify(&Self::stats_text(), "info")?;
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:compact-doctor")
                .description("Check compaction settings and leftovers")
                .run(|ext, _args, cx| async move {
                    let auto = control::settings().ok().and_then(|s| {
                        s.pointer("/compaction/enabled")
                            .and_then(Value::as_bool)
                            .or(Some(true))
                    });
                    let (ok, text) = card::doctor(&ext.cfg(), auto, control::model().is_some());
                    if cx.has_ui() {
                        ui::notify(&text, if ok { "info" } else { "warning" })?;
                    }
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:compact-help")
                .description("Show compactor help")
                .run(|_ext, _args, cx| async move {
                    if cx.has_ui() {
                        ui::notify(card::HELP, "info")?;
                    }
                    Ok(())
                }),
        );
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Compactor);

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn compact_then() {
        assert_eq!(
            compact_then_prompt("/unipi:compact-then"),
            Some(String::new())
        );
        assert_eq!(
            compact_then_prompt("/unipi:compact-then  next task \n more"),
            Some("next task \n more".into())
        );
        assert_eq!(compact_then_prompt("/unipi:compact-thenx"), None);
        assert_eq!(compact_then_prompt("hello"), None);
        assert_eq!(
            join_instructions(&[Some(" a "), None, Some(""), Some("b")]),
            Some("a\n\nb".into())
        );
        let ops = ops_of(Some(
            &json!({"fileOps": {"read": ["a"], "written": ["b"], "edited": ["c"]}}),
        ));
        assert!(ops.read.contains("a") && ops.modified.contains("b") && ops.modified.contains("c"));
    }
}
