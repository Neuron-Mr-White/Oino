//! `unipi.fusion` — a strong lead paired with a persistent cheaper sidekick, and the model
//! picker (unipi `packages/fusion`, unipi-port §3.13).
//!
//! | unipi | here |
//! |---|---|
//! | sidekick = one persistent `pi --mode rpc` child per lead session | one persistent nested agent (`persistence: file` under `/ext-session/<sid>/sidekick`, role `fusion`) |
//! | `sidekick {message, block?}`; a call while running = interrupt (steer) | tool |
//! | `read_subagent` reader for sidekick handoffs | service `subagent-reader.read`, registered with `unipi.subagents` |
//! | `sidekick-step` entries, `sidekick-completion` messages | entries/messages + renderers |
//! | lead policy section, edit nudge (once per turn), bash nudge (every 4 non-trivial) | `before_agent_start` section `fusion-lead-policy`; `tool_result` annotations with provenance |
//! | `/fusion` picker, `Alt+Enter` startup default, `/model` autocomplete boost | sequential dialogs ([`Fusion::picker`]); startup pair in the preset; `complete-input` |
//! | `/fusion-stats`, preset editor runner | command; hidden command `fusion-preset` |
//! | presets (global + project), `FUSION_STATUS` sticky, wake widget, wait source | [`preset`]; topic `unipi:fusion:status`; widget `fusion-sidekick-wake`; `wait-source` |
//! | `unipi.fusion.api` | services `fusion.status`, `fusion.set-pair` |
//! | settings `fusion` (`startup.*`, `default.*`) | namespace |

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

/// Child-agent runtime shared with `unipi.subagents` (same source file; a crate dependency
/// would link that component's exports into this one).
#[path = "../../subagents/src/child.rs"]
pub mod child;
pub mod policy;
pub mod preset;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::child::{Delivery, Done, Progress, Report, Spec, Usage};
use futures::StreamExt;
use oino_extension::agents::Agent;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::services::point;
use unipi_kit::settings::{Field, Kind, Namespace, Section, Store, plain};
use unipi_kit::topics::{self, FusionStatus};

use crate::policy::Identity;
use crate::preset::{Active, Pair, Preset, Price};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.fusion";
/// Settings namespace.
pub const NAMESPACE: &str = "fusion";
/// `/fusion`: the model / Fusion pair picker (unipi `/unipi:model`; renamed because the core
/// `/model` built-in wins).
pub const MODEL_COMMAND: &str = "fusion";
/// Hidden preset editor runner.
pub const PRESET_COMMAND: &str = "fusion-preset";
/// Wake widget key.
pub const WAKE_WIDGET: &str = "fusion-sidekick-wake";
const SIDEKICK_DESCRIPTION: &str = "Hand off work to your persistent sidekick subagent (one per session; context and shells persist across handoffs; runs on the same machine). block:true (default) waits and returns the report. block:false returns immediately and the report arrives later as a <subagent_completion_notification>. Calling again while a handoff is running injects the message as an interrupt rather than starting a second sidekick.";
const NOT_ACTIVE: &str = "Fusion is not active — pick a Fusion pair with /fusion.";
const STARTUP_THINKING: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];

struct Handoff {
    id: String,
    started_at: i64,
    progress: RefCell<Progress>,
    done: Rc<Done<Report>>,
}

struct Sidekick {
    agent: Rc<Agent>,
    handoffs: RefCell<Vec<Rc<Handoff>>>,
    usage: RefCell<Usage>,
}

impl Sidekick {
    fn latest(&self) -> Option<Rc<Handoff>> {
        self.handoffs.borrow().last().cloned()
    }
    fn busy(&self) -> bool {
        self.latest().is_some_and(|h| h.done.get().is_none())
    }
    fn report(&self, id: &str) -> Option<Report> {
        self.handoffs
            .borrow()
            .iter()
            .find(|h| h.id == id)
            .and_then(|h| h.done.get())
    }
    fn total_tool_calls(&self) -> u32 {
        self.handoffs
            .borrow()
            .iter()
            .map(|h| h.progress.borrow().tool_calls)
            .sum()
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent per-session flags from unipi"
)]
#[derive(Default)]
struct State {
    active: Option<Active>,
    sidekick: Option<Rc<Sidekick>>,
    lead_tool_calls: u64,
    edit_nudged: bool,
    bash_streak: u32,
    bash_nudged: bool,
    wake: bool,
    background: HashSet<String>,
    registered: bool,
}

/// The extension.
#[derive(Default)]
pub struct Fusion {
    settings: Store,
    sessions: SessionMap<State>,
    delivery: Delivery,
}

/// One available model (`control.models-list`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelInfo {
    /// `provider/id`.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Price per 1M (when the catalogue has one).
    pub cost: Option<Price>,
    /// Supports reasoning.
    pub reasoning: bool,
}

/// Models from the host list.
#[must_use]
pub fn models_from(list: &[Value]) -> Vec<ModelInfo> {
    list.iter()
        .map(|m| {
            let g = |k: &str| {
                m.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let c = m.get("cost");
            let n = |k: &str| {
                c.and_then(|c| c.get(k))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            let cost = Price {
                input: n("input"),
                cached_input: n("cacheRead"),
                output: n("output"),
            };
            ModelInfo {
                key: format!("{}/{}", g("provider"), g("id")),
                name: if g("name").is_empty() {
                    g("id")
                } else {
                    g("name")
                },
                cost: (cost.input > 0.0 || cost.cached_input > 0.0 || cost.output > 0.0)
                    .then_some(cost),
                reasoning: m.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
            }
        })
        .collect()
}

/// Picker row order: active pinned, Fusion row, recent, preset models, every other model.
#[must_use]
pub fn picker_rows(models: &[ModelInfo], preset: &Preset, active: Option<&Active>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut add = |k: &str| {
        if models.iter().any(|m| m.key == k) && !keys.iter().any(|x| x == k) {
            keys.push(k.to_owned());
        }
    };
    if let Some(Active::Single { model }) = active {
        add(model);
    }
    for k in preset
        .recent
        .iter()
        .chain(&preset.lead)
        .chain(&preset.sidekick)
    {
        add(k);
    }
    for m in models {
        add(&m.key);
    }
    keys
}

fn price_text(p: Option<&Price>) -> String {
    p.map_or_else(
        || "no pricing".into(),
        |c| {
            format!(
                "${} / ${} / ${} per 1M (in / cached / out)",
                trim_num(c.input),
                trim_num(c.cached_input),
                trim_num(c.output)
            )
        },
    )
}

fn trim_num(x: f64) -> String {
    let s = format!("{x:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The `/model` hint: `/fusion` offered while typing `/m`, `/mo`, … `/model`, ranked after the
/// core `/model` built-in.
#[must_use]
pub fn boost_completion(text: &str) -> Option<Value> {
    let t = text.to_ascii_lowercase();
    let ok = ["/m", "/mo", "/mod", "/mode", "/model"].contains(&t.as_str());
    ok.then(|| json!([{"source": "fusion", "items": [{"value": format!("/{MODEL_COMMAND}"), "label": format!("/{MODEL_COMMAND}"), "description": "Pick a model or a Fusion lead+sidekick pair", "sortTier": 1}]}]))
}

/// unipi `sidekickWakeText`.
#[must_use]
pub fn wake_text(tool_calls: u32, elapsed_ms: i64) -> String {
    format!(
        "sidekick working · {tool_calls} tool calls · {} — resumes automatically when done",
        duration(elapsed_ms)
    )
}

/// unipi `duration` (`12s`, `3m04s`, `1h02m`).
#[must_use]
pub fn duration(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// unipi `reportText`.
#[must_use]
pub fn report_text(r: &Report) -> String {
    format!(
        "{}\n\n--- sidekick {} · {} · {} tool calls · {} · in {} / out {} tokens",
        r.text,
        r.id,
        status_name(r),
        r.tool_calls,
        duration(r.duration_ms),
        r.usage.input,
        r.usage.output
    )
}

fn status_name(r: &Report) -> &'static str {
    match r.status {
        child::ReportStatus::Completed => "completed",
        child::ReportStatus::Aborted => "aborted",
        child::ReportStatus::Error => "error",
        child::ReportStatus::Interrupted => "interrupted",
    }
}

/// unipi `progressText`.
#[must_use]
pub fn progress_text(p: &Progress, elapsed_ms: i64) -> String {
    let tools = if p.recent_tools.is_empty() {
        String::new()
    } else {
        format!(
            "\n{}",
            p.recent_tools
                .iter()
                .map(|t| format!("  {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let tail = if p.text_tail.is_empty() {
        String::new()
    } else {
        format!("\n  {}", p.text_tail)
    };
    format!(
        "◆ sidekick working · {} tool calls · {}{tools}{tail}",
        p.tool_calls,
        duration(elapsed_ms)
    )
}

fn result(text: impl Into<String>, details: Value, is_error: bool) -> ToolResult {
    let mut r = ToolResult::text(text).with_details(details);
    r.is_error = is_error;
    r
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

fn global_path() -> PathBuf {
    unipi_kit::paths::global("preset.json")
}

fn project_path() -> PathBuf {
    PathBuf::from("/project/.oino/fusion-preset.json")
}

fn read_raw(p: &std::path::Path) -> Option<Value> {
    unipi_kit::paths::read_json(p)
}

/// Settings schema (unipi `registerSettings({namespace: "fusion"})`).
#[must_use]
pub fn namespace() -> Namespace {
    let model = |key: &str, label: &str, empty: &str, desc: &str| {
        Field::model(key, label).description(desc).with_kind(|k| {
            if let Kind::Model {
                filter,
                empty_label,
                empty_option,
                ..
            } = k
            {
                *filter = Some(json!({"capability": "text"}));
                *empty_label = Some(empty.to_owned());
                if empty != "Oino default" {
                    *empty_option = Some(empty.to_owned());
                }
            }
        })
    };
    Namespace::new(NAMESPACE, "Fusion", EXTENSION_ID)
        .defaults(json!({"default": {"lead": "", "sidekick": ""}, "startup": {"model": ""}}))
        .section(
            Section::new("startup", "Startup")
                .description("The model Oino opens with. /fusion → \"use at startup\" sets the same thing (on the Fusion row it starts in Fusion).")
                .field(model("startup.model", "Startup model", "Oino default", "The model Oino opens with. /fusion \"use at startup\" sets the same thing.").scope(unipi_kit::settings::Scope::Global))
                .field(Field::enumeration("startup.thinking", "Startup thinking level", STARTUP_THINKING.iter().map(|s| plain(s)).collect()).scope(unipi_kit::settings::Scope::Global).description("Thinking level the session starts at.")),
        )
        .section(
            Section::new("default-pair", "Default pair")
                .description("Used when the Fusion row is confirmed without editing")
                .field(model("default.lead", "Lead", "picker default", "Lead model of the pair used when the Fusion row is confirmed unedited."))
                .field(model("default.sidekick", "Sidekick", "picker default", "Sidekick model of the pair used when the Fusion row is confirmed unedited."))
                .field(Field::action("presets.editor", "Edit fusion presets…", PRESET_COMMAND).description("Curate the lead and sidekick lists and the defaults.")),
        )
}

impl Fusion {
    fn preset(&self) -> (Preset, bool) {
        let mut p = preset::merge(
            Preset::default(),
            preset::parse(&read_raw(&global_path()).unwrap_or_default()),
        );
        let project = read_raw(&project_path());
        let has_project = project.is_some();
        if let Some(raw) = project {
            p = preset::merge(p, preset::parse(&raw));
        }
        let v = self.settings.get(NAMESPACE);
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        };
        if let Some(l) = s("default.lead") {
            p.default.lead = Some(l);
        }
        if let Some(sk) = s("default.sidekick") {
            p.default.sidekick = Some(sk);
        }
        (p, has_project)
    }

    #[allow(
        clippy::unused_self,
        reason = "state lives in files; kept on the extension for one call site style"
    )]
    fn save_runtime(
        &self,
        effort: &std::collections::BTreeMap<String, String>,
        recent: &[String],
        active: Option<&Active>,
    ) {
        let path = global_path();
        let raw = read_raw(&path).unwrap_or_else(|| json!({}));
        let _ = unipi_kit::paths::write_json(
            &path,
            &preset::with_runtime(&raw, effort, recent, active),
        );
    }

    #[allow(
        clippy::unused_self,
        reason = "state lives in files; kept on the extension for one call site style"
    )]
    fn save_curation(
        &self,
        project: bool,
        lead: &[String],
        sidekick: &[String],
        default: &Pair,
    ) -> PathBuf {
        let path = if project {
            project_path()
        } else {
            global_path()
        };
        let raw = read_raw(&path).unwrap_or_else(|| json!({}));
        let _ = unipi_kit::paths::write_json(
            &path,
            &preset::with_curation(&raw, lead, sidekick, default),
        );
        path
    }

    fn models() -> Vec<ModelInfo> {
        models_from(&control::models().unwrap_or_default())
    }

    fn name_of(models: &[ModelInfo], key: &str) -> String {
        models.iter().find(|m| m.key == key).map_or_else(
            || preset::split_key(key).map_or(key, |(_, id)| id).to_owned(),
            |m| m.name.clone(),
        )
    }

    fn identity(&self, sid: &str) -> Identity {
        let models = Self::models();
        match self.sessions.with(sid, |s| s.active.clone()) {
            Some(Active::Fusion {
                lead,
                sidekick,
                lead_effort,
                sidekick_effort,
            }) => Identity {
                lead_name: Self::name_of(&models, &lead),
                lead_effort: preset::effort_label(lead_effort.as_deref().unwrap_or("medium")),
                sidekick_name: Self::name_of(&models, &sidekick),
                sidekick_effort: preset::effort_label(
                    sidekick_effort.as_deref().unwrap_or("medium"),
                ),
            },
            _ => Identity::default(),
        }
    }

    fn fusion_on(&self, sid: &str) -> bool {
        self.sessions
            .with(sid, |s| matches!(s.active, Some(Active::Fusion { .. })))
    }

    fn savings(&self, sid: &str) -> Option<(f64, f64, f64)> {
        let (Some(Active::Fusion { lead, sidekick, .. }), Some(sk)) = self
            .sessions
            .with(sid, |s| (s.active.clone(), s.sidekick.clone()))
        else {
            return None;
        };
        let models = Self::models();
        let (p, _) = self.preset();
        let cost = |k: &str| {
            p.prices
                .get(k)
                .copied()
                .or_else(|| models.iter().find(|m| m.key == k).and_then(|m| m.cost))
        };
        let usage = *sk.usage.borrow();
        Some(policy::estimate_savings(
            &usage,
            cost(&lead).as_ref(),
            cost(&sidekick).as_ref(),
        ))
    }

    fn publish_status(&self, sid: &str) {
        let (active, sk, lead_calls) = self.sessions.with(sid, |s| {
            (s.active.clone(), s.sidekick.clone(), s.lead_tool_calls)
        });
        let status = match active {
            Some(Active::Fusion {
                lead,
                sidekick,
                lead_effort,
                sidekick_effort,
            }) => {
                let models = Self::models();
                Some(FusionStatus {
                    lead_name: Self::name_of(&models, &lead),
                    lead_key: Some(lead),
                    sidekick_name: Self::name_of(&models, &sidekick),
                    sidekick_key: Some(sidekick),
                    lead_effort: lead_effort.unwrap_or_default(),
                    sidekick_effort: sidekick_effort.unwrap_or_default(),
                    saved_usd: self.savings(sid).map(|s| s.2),
                    busy: Some(sk.as_ref().is_some_and(|k| k.busy())),
                    lead_tool_calls: Some(lead_calls),
                    sidekick_tool_calls: Some(sk.map_or(0, |k| u64::from(k.total_tool_calls()))),
                })
            }
            _ => None,
        };
        unipi_kit::bus::emit(&status);
        self.publish_wake(sid);
    }

    fn publish_wake(&self, sid: &str) {
        let sk = self.sessions.with(sid, |s| s.sidekick.clone());
        let busy = sk.as_ref().is_some_and(|k| k.busy());
        let want = busy && control::is_idle();
        let was = self
            .sessions
            .with(sid, |s| std::mem::replace(&mut s.wake, want));
        if want {
            let h = sk.and_then(|k| k.latest());
            let text = h
                .map(|h| {
                    wake_text(
                        h.progress.borrow().tool_calls,
                        timers::now_ms() - h.started_at,
                    )
                })
                .unwrap_or_default();
            let _ = ui::fire(
                &json!({"method": "setWidget", "widgetKey": WAKE_WIDGET, "widgetLines": [text], "widgetPlacement": "aboveEditor", "widgetSpec": {"t": "stack", "direction": "horizontal", "gap": 1, "children": [{"t": "spinner"}, {"t": "text", "value": text}]}}),
            );
        } else if was {
            let _ = ui::fire(&json!({"method": "setWidget", "widgetKey": WAKE_WIDGET}));
        }
    }

    fn sync_tools(&self, cx: &Ctx) {
        let want = self.fusion_on(cx.session_id());
        let mut tools = control::active_tools();
        let has = tools.iter().any(|t| t == "sidekick");
        if has != want {
            if want {
                tools.push("sidekick".into());
            } else {
                tools.retain(|t| t != "sidekick");
            }
            let _ = control::set_active_tools(&tools);
        }
    }

    async fn register_reader(&self, cx: &Ctx) {
        if self.sessions.with(cx.session_id(), |s| {
            std::mem::replace(&mut s.registered, true)
        }) {
            return;
        }
        let _ = services::call(
            Some(unipi_kit::suite::pkg::SUBAGENTS.id),
            point::SUBAGENTS_REGISTER_EXTERNAL,
            &json!({"owner": EXTENSION_ID}),
            None,
        )
        .await;
    }

    fn stop_runtime(&self, sid: &str) {
        let sk = self.sessions.with(sid, |s| {
            s.lead_tool_calls = 0;
            s.background.clear();
            s.sidekick.take()
        });
        if let Some(sk) = sk {
            sk.agent.abort();
        }
        if self.sessions.with(sid, |s| std::mem::take(&mut s.wake)) {
            let _ = ui::fire(&json!({"method": "setWidget", "widgetKey": WAKE_WIDGET}));
        }
    }

    // ── sidekick runtime ───────────────────────────────────────────────────

    async fn runtime(&self, sid: &str) -> std::result::Result<Rc<Sidekick>, String> {
        if let Some(sk) = self.sessions.with(sid, |s| s.sidekick.clone()) {
            return Ok(sk);
        }
        let Some(Active::Fusion {
            sidekick,
            sidekick_effort,
            ..
        }) = self.sessions.with(sid, |s| s.active.clone())
        else {
            return Err(NOT_ACTIVE.into());
        };
        let dir = unipi_kit::paths::session(sid, "sidekick");
        let _ = std::fs::create_dir_all(&dir);
        let existing = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "jsonl"));
        let spec = Spec {
            label: "Sidekick".into(),
            model: Some(sidekick),
            thinking: Some(sidekick_effort.unwrap_or_else(|| "medium".into())),
            append_system_prompt: Some(policy::sidekick_prompt(&self.identity(sid))),
            allow: None,
            deny: vec![
                "sidekick".into(),
                "read_subagent".into(),
                "run_subagent".into(),
            ],
            seed_session_file: existing
                .and_then(|p| unipi_kit::paths::host_path(&p))
                .map(|p| p.to_string_lossy().into_owned()),
            session_dir: unipi_kit::paths::host_path(&dir)
                .map(|p| p.to_string_lossy().into_owned()),
            child_role: "fusion".into(),
            forward_ui: true,
            max_depth: 1,
        };
        let agent = Agent::spawn(&spec.to_json())
            .await
            .map_err(|e| e.to_string())?;
        let sk = Rc::new(Sidekick {
            agent: Rc::new(agent),
            handoffs: RefCell::new(Vec::new()),
            usage: RefCell::new(Usage::default()),
        });
        self.sessions
            .with(sid, |s| s.sidekick = Some(Rc::clone(&sk)));
        Ok(sk)
    }

    /// Starts a handoff, or interrupts the running one.
    async fn handoff(
        &'static self,
        sid: &str,
        sk: &Rc<Sidekick>,
        message: &str,
    ) -> std::result::Result<Rc<Handoff>, String> {
        if let Some(h) = sk.latest().filter(|h| h.done.get().is_none()) {
            sk.agent.steer(message).await.map_err(|e| e.to_string())?;
            return Ok(h);
        }
        let h = Rc::new(Handoff {
            id: unipi_kit::fmt::random_hex(8),
            started_at: timers::now_ms(),
            progress: RefCell::new(Progress::default()),
            done: Done::new(),
        });
        sk.handoffs.borrow_mut().push(Rc::clone(&h));
        let (sid, sk2, h2, msg) = (
            sid.to_owned(),
            Rc::clone(sk),
            Rc::clone(&h),
            message.to_owned(),
        );
        spawn(async move { self.drive(&sid, &sk2, &h2, &msg).await });
        Ok(h)
    }

    async fn drive(&self, sid: &str, sk: &Rc<Sidekick>, h: &Rc<Handoff>, message: &str) {
        let mut events = sk.agent.events();
        let mut error = sk.agent.prompt(message).await.err().map(|e| e.to_string());
        let mut had_bg = false;
        while error.is_none() {
            let Some(ev) = events.next().await else {
                error = Some("Sidekick process exited".into());
                break;
            };
            let (steps, fin) = {
                let mut p = h.progress.borrow_mut();
                let before = p.usage;
                let steps = p.apply(&ev, timers::now_ms());
                let after = p.usage;
                let mut u = sk.usage.borrow_mut();
                u.input += after.input - before.input;
                u.output += after.output - before.output;
                u.cache_read += after.cache_read - before.cache_read;
                u.cache_write += after.cache_write - before.cache_write;
                u.cost += after.cost - before.cost;
                had_bg |= p.open_bg_tasks > 0;
                (steps, p.finishable())
            };
            for step in steps {
                let mut data = serde_json::to_value(&step).unwrap_or_default();
                data["group"] = json!(format!("sidekick:{}", h.id));
                data["label"] = json!("Sidekick");
                let _ = session::append_entry("sidekick-step", Some(&data));
            }
            self.publish_status(sid);
            if fin {
                if had_bg {
                    timers::sleep(Duration::from_millis(child::SETTLE_GRACE_MS)).await;
                    if !h.progress.borrow().finishable() {
                        continue;
                    }
                }
                break;
            }
        }
        let text = sk.agent.last_assistant_text();
        let report = h.progress.borrow_mut().report(
            &h.id,
            false,
            text,
            error,
            timers::now_ms() - h.started_at,
        );
        h.done.set(report.clone());
        self.sessions.with(sid, |s| s.background.remove(&h.id));
        self.publish_status(sid);
        if self.delivery.should_deliver(&h.id) && self.sessions.with(sid, |_| true) {
            Self::deliver(&report).await;
        }
    }

    async fn deliver(report: &Report) {
        let msg = session::CustomMessage {
            custom_type: "sidekick-completion".into(),
            content: json!(format!(
                "<subagent_completion_notification agent_id=\"{}\" status=\"{}\">\n{}\n</subagent_completion_notification>",
                report.id,
                status_name(report),
                report.text
            )),
            display: true,
            details: serde_json::to_value(report).ok(),
        };
        let _ = session::send_message(
            &msg,
            session::SendOptions {
                trigger_turn: true,
                deliver_as: Some(session::Delivery::FollowUp),
                provenance: None,
            },
        )
        .await;
    }

    /// Detaches a waiter: delivery armed, tracked as a background handoff.
    fn detach(&'static self, sid: &str, h: &Rc<Handoff>) {
        self.sessions
            .with(sid, |s| s.background.insert(h.id.clone()));
        if self.delivery.detach(&h.id)
            && let Some(r) = h.done.get()
        {
            self.sessions.with(sid, |s| s.background.remove(&h.id));
            if self.delivery.should_deliver(&h.id) {
                spawn(async move { Self::deliver(&r).await });
            }
        }
    }

    async fn wait(
        &self,
        sk: &Rc<Sidekick>,
        h: &Rc<Handoff>,
        cx: &ToolCtx,
        timeout_ms: i64,
    ) -> Waited {
        self.delivery.attach(&h.id);
        let started = timers::now_ms();
        let mut last_key = String::new();
        loop {
            if let Some(r) = h.done.get() {
                return Waited::Report(r);
            }
            if cx.is_cancelled() {
                sk.agent.abort();
                return Waited::Aborted;
            }
            if control::has_pending_messages() {
                return Waited::Interrupted(progress_text(
                    &h.progress.borrow(),
                    timers::now_ms() - h.started_at,
                ));
            }
            if timers::now_ms() - started >= timeout_ms {
                return Waited::Timeout;
            }
            let (key, text) = {
                let p = h.progress.borrow();
                (
                    format!(
                        "{}|{}|{}|{}",
                        p.tool_calls,
                        p.recent_tools.join("|"),
                        p.text_tail,
                        p.events.len()
                    ),
                    progress_text(&p, timers::now_ms() - h.started_at),
                )
            };
            if key != last_key {
                last_key = key;
                cx.progress(&ToolResult::text(text).with_details(json!({"progress": {"toolCalls": h.progress.borrow().tool_calls, "startedAt": h.started_at}, "id": h.id})));
            }
            timers::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn sidekick_tool(&'static self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let sid = cx.session_id().to_owned();
        if !self.fusion_on(&sid) {
            return Ok(result(NOT_ACTIVE, json!({}), true));
        }
        let sk = match self.runtime(&sid).await {
            Ok(s) => s,
            Err(e) => return Ok(result(e, json!({}), true)),
        };
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let h = match self.handoff(&sid, &sk, message).await {
            Ok(h) => h,
            Err(e) => return Ok(result(format!("Handoff failed: {e}"), json!({}), true)),
        };
        self.publish_status(&sid);
        if args.get("block").and_then(Value::as_bool) == Some(false) {
            self.detach(&sid, &h);
            return Ok(result(
                format!(
                    "Handoff {0} started in the background. You will receive a <subagent_completion_notification agent_id=\"{0}\"> when it finishes; use read_subagent to wait.",
                    h.id
                ),
                json!({"background": true, "id": h.id}),
                false,
            ));
        }
        Ok(self.finish_wait(&sid, &sk, &h, cx, 2_700_000, false).await)
    }

    async fn finish_wait(
        &'static self,
        sid: &str,
        sk: &Rc<Sidekick>,
        h: &Rc<Handoff>,
        cx: &ToolCtx,
        timeout_ms: i64,
        reader: bool,
    ) -> ToolResult {
        let owner = |mut d: Value| {
            if reader {
                d["owner"] = json!("fusion");
            }
            d
        };
        let waited = self.wait(sk, h, cx, timeout_ms).await;
        if let Waited::Report(r) = &waited {
            self.delivery.consume(&h.id);
            self.publish_status(sid);
            return result(
                report_text(r),
                owner(serde_json::to_value(r).unwrap_or_default()),
                r.status != child::ReportStatus::Completed,
            );
        }
        self.detach(sid, h);
        self.publish_status(sid);
        let progress = || {
            owner(
                json!({"progress": {"toolCalls": h.progress.borrow().tool_calls, "startedAt": h.started_at}, "id": h.id}),
            )
        };
        let ptext = progress_text(&h.progress.borrow(), timers::now_ms() - h.started_at);
        match waited {
            Waited::Aborted if reader => {
                result(format!("Handoff {} aborted.", h.id), json!({}), true)
            }
            Waited::Aborted => result(
                format!("{ptext}\nHandoff {} aborted.", h.id),
                json!({}),
                true,
            ),
            Waited::Interrupted(t) if reader => result(
                format!(
                    "A user message arrived while the sidekick (agent_id {}) was working.\n{t}",
                    h.id
                ),
                progress(),
                false,
            ),
            Waited::Interrupted(t) => result(
                format!(
                    "A user message arrived while the sidekick (agent_id {0}) was working. The handoff continues in the background. Act on the user's message first, then call read_subagent({{agent_id:\"{0}\", block:true}}) to collect the report or sidekick({{message}}) to redirect it.\n{t}",
                    h.id
                ),
                progress(),
                false,
            ),
            _ => result(
                format!("Handoff {} is still running.\n{ptext}", h.id),
                progress(),
                false,
            ),
        }
    }

    /// `subagent-reader.read` (the shared `read_subagent` tool, served by `unipi.subagents`).
    async fn reader(&'static self, p: &Value, cx: &Ctx) -> Value {
        let sid = cx.session_id().to_owned();
        let sk = self.sessions.with(&sid, |s| s.sidekick.clone());
        let id = p
            .get("agent_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if p.get("probe").and_then(Value::as_bool) == Some(true) {
            let owns = id.as_deref().is_some_and(|i| {
                sk.as_ref()
                    .is_some_and(|k| k.handoffs.borrow().iter().any(|h| h.id == i))
            });
            let latest = sk
                .and_then(|k| k.latest())
                .map(|h| json!({"id": h.id, "startedAt": h.started_at}));
            return json!({"owns": owns, "latest": latest});
        }
        let tr = |r: ToolResult| serde_json::to_value(r).unwrap_or_default();
        let Some(sk) = sk.filter(|_| self.fusion_on(&sid)) else {
            return tr(result(NOT_ACTIVE, json!({}), true));
        };
        let Some(latest) = sk.latest() else {
            return tr(result("No sidekick handoff has run yet.", json!({}), true));
        };
        let id = id.unwrap_or_else(|| latest.id.clone());
        if let Some(r) = sk.report(&id) {
            self.delivery.consume(&id);
            let mut d = serde_json::to_value(&r).unwrap_or_default();
            d["owner"] = json!("fusion");
            return tr(result(
                report_text(&r),
                d,
                r.status != child::ReportStatus::Completed,
            ));
        }
        if id != latest.id {
            return tr(result(
                format!("No sidekick handoff found for {id}."),
                json!({}),
                true,
            ));
        }
        if p.get("block").and_then(Value::as_bool) != Some(true) {
            let t = progress_text(
                &latest.progress.borrow(),
                timers::now_ms() - latest.started_at,
            );
            return tr(result(
                format!("Handoff {id} is still running.\n{t}"),
                json!({"id": id, "owner": "fusion"}),
                false,
            ));
        }
        #[allow(clippy::cast_possible_truncation, reason = "≤ 600 s")]
        let timeout = (p
            .get("timeout")
            .and_then(Value::as_f64)
            .unwrap_or(30.0)
            .min(600.0)
            * 1000.0) as i64;
        let tcx = ToolCtx {
            cx: cx.clone(),
            tool_call_id: String::new(),
            cancel: oino_extension::context::Cancel::default(),
        };
        tr(self
            .finish_wait(&sid, &sk, &latest, &tcx, timeout, true)
            .await)
    }

    // ── lead policy and nudges ─────────────────────────────────────────────

    fn before_agent_start(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        self.sessions.with(cx.session_id(), |s| s.bash_streak = 0);
        self.sync_tools(cx);
        let mut opts = ev
            .get("systemPromptOptions")
            .cloned()
            .filter(Value::is_object)?;
        if !opts.get("sections").is_some_and(Value::is_object) {
            opts["sections"] = json!({});
        }
        let sections = opts.get_mut("sections").and_then(Value::as_object_mut)?;
        if self.fusion_on(cx.session_id()) {
            sections.insert(
                "fusion-lead-policy".into(),
                json!(policy::lead_policy(&self.identity(cx.session_id()))),
            );
        } else {
            sections.remove("fusion-lead-policy");
        }
        Some(json!({"systemPromptOptions": opts}))
    }

    fn tool_result(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let sid = cx.session_id();
        if !self.fusion_on(sid) {
            return None;
        }
        let tool = ev
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if tool == "sidekick" || tool == "read_subagent" {
            self.sessions.with(sid, |s| {
                s.bash_streak = 0;
                s.bash_nudged = false;
            });
            return None;
        }
        self.sessions.with(sid, |s| s.lead_tool_calls += 1);
        self.publish_status(sid);
        let annotate = |text: &str, title: &str, synopsis: &str| {
            let mut content = ev
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            content.push(json!({"type": "text", "text": text}));
            let mut details = ev
                .get("details")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            details["oinoProvenance"] = json!({"extensionId": EXTENSION_ID, "source": "Fusion", "title": title, "synopsis": synopsis, "severity": "warning", "delivery": "boundary"});
            Some(json!({"content": content, "details": details}))
        };
        if tool == "edit" || tool == "write" {
            if self
                .sessions
                .with(sid, |s| std::mem::replace(&mut s.edit_nudged, true))
            {
                return None;
            }
            return annotate(
                policy::EDIT_NUDGE,
                "Delegate edits",
                "Direct edit instead of sidekick delegation",
            );
        }
        if tool != "bash" {
            return None;
        }
        let command = ev
            .payload
            .pointer("/input/command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if policy::is_trivial_shell(command) {
            return None;
        }
        let streak = self.sessions.with(sid, |s| {
            if s.bash_nudged {
                return None;
            }
            s.bash_streak += 1;
            if s.bash_streak < policy::BASH_NUDGE_EVERY {
                return None;
            }
            s.bash_nudged = true;
            Some(std::mem::take(&mut s.bash_streak))
        })?;
        annotate(
            &policy::bash_nudge(streak),
            "Delegate shell work",
            "Non-trivial shell work since last handoff",
        )
    }

    // ── picker ─────────────────────────────────────────────────────────────

    /// Applies a selection (unipi `applyResult`).
    fn apply(
        &self,
        sid: &str,
        sel: &Active,
        effort_map: &std::collections::BTreeMap<String, String>,
        silent: bool,
    ) -> std::result::Result<(), String> {
        let models = Self::models();
        let target = match sel {
            Active::Single { model } => model.clone(),
            Active::Fusion { lead, .. } => lead.clone(),
        };
        let Some((provider, id)) =
            preset::split_key(&target).filter(|_| models.iter().any(|m| m.key == target))
        else {
            return Err(format!(
                "Model {target} is not available. Run /login or fix the preset."
            ));
        };
        if !control::set_model(provider, id).map_err(|e| e.to_string())? {
            return Err(format!("Could not switch to {target}."));
        }
        let prev = self.sessions.with(sid, |s| s.active.clone());
        let same_pair = matches!((&prev, sel), (Some(Active::Fusion { lead: a, sidekick: b, .. }), Active::Fusion { lead: c, sidekick: d, .. }) if a == c && b == d);
        if !same_pair {
            self.stop_runtime(sid);
        } else if let (
            Some(Active::Fusion {
                sidekick_effort: old,
                ..
            }),
            Active::Fusion {
                sidekick_effort: new,
                ..
            },
        ) = (&prev, sel)
            && old != new
            && let Some(sk) = self.sessions.with(sid, |s| s.sidekick.clone())
        {
            let _ = sk.agent.set_thinking(new.as_deref().unwrap_or("medium"));
        }
        let effort = match sel {
            Active::Single { model } => effort_map
                .get(model)
                .cloned()
                .unwrap_or_else(|| "medium".into()),
            Active::Fusion { lead_effort, .. } => {
                lead_effort.clone().unwrap_or_else(|| "medium".into())
            }
        };
        let _ = control::set_thinking_level(&effort);
        self.sessions.with(sid, |s| s.active = Some(sel.clone()));
        let (p, has_project) = self.preset();
        let recent = preset::push_recent(&p.recent, &target);
        if let Active::Fusion { lead, sidekick, .. } = sel {
            let mut l = p.lead.clone();
            if !l.contains(lead) {
                l.insert(0, lead.clone());
            }
            let mut sk = p.sidekick.clone();
            if !sk.contains(sidekick) {
                sk.insert(0, sidekick.clone());
            }
            self.save_curation(
                has_project,
                &l,
                &sk,
                &Pair {
                    lead: Some(lead.clone()),
                    sidekick: Some(sidekick.clone()),
                },
            );
        }
        self.save_runtime(effort_map, &recent, Some(sel));
        if !silent {
            let label = match sel {
                Active::Single { model } => format!(
                    "{} · {}",
                    Self::name_of(&models, model),
                    preset::effort_label(&effort)
                ),
                Active::Fusion {
                    lead,
                    sidekick,
                    sidekick_effort,
                    ..
                } => format!(
                    "Fusion · {} {} ◆ {} {}",
                    Self::name_of(&models, lead),
                    preset::effort_label(&effort),
                    Self::name_of(&models, sidekick),
                    preset::effort_label(sidekick_effort.as_deref().unwrap_or("medium"))
                ),
            };
            let _ = ui::notify(&label, "info");
        }
        Ok(())
    }

    async fn pick_key(
        title: &str,
        keys: &[String],
        models: &[ModelInfo],
        default: Option<&str>,
    ) -> Option<String> {
        let mut ordered: Vec<String> = default
            .map(ToOwned::to_owned)
            .into_iter()
            .filter(|d| keys.contains(d))
            .collect();
        ordered.extend(keys.iter().filter(|k| Some(k.as_str()) != default).cloned());
        let labels: Vec<String> = ordered
            .iter()
            .map(|k| format!("{} ({k})", Self::name_of(models, k)))
            .collect();
        let details: Vec<Value> = ordered.iter().map(|k| json!({"label": k, "description": price_text(models.iter().find(|m| &m.key == k).and_then(|m| m.cost.as_ref()))})).collect();
        let answer = ui::dialog(&json!({"method": "select", "title": title, "options": labels, "optionDetails": details})).await.ok()?;
        let v = answer.get("value").and_then(Value::as_str)?;
        labels
            .iter()
            .position(|l| l == v)
            .and_then(|i| ordered.get(i).cloned())
    }

    async fn pick_effort(title: &str, current: &str) -> Option<String> {
        let labels: Vec<String> = preset::EFFORT_LEVELS
            .iter()
            .map(|e| {
                if *e == current {
                    format!("{} (current)", preset::effort_label(e))
                } else {
                    preset::effort_label(e)
                }
            })
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let v = ui::select(title, &refs).await.ok()??;
        labels
            .iter()
            .position(|l| *l == v)
            .map(|i| preset::EFFORT_LEVELS[i].to_owned())
    }

    /// `/fusion` (unipi Devin-style picker, as a dialog sequence).
    #[allow(
        clippy::too_many_lines,
        reason = "one dialog sequence, as the unipi picker"
    )]
    async fn picker(&self, cx: &CommandCtx) -> Result<()> {
        if !cx.has_ui() {
            return Err(Error::msg(format!(
                "/{MODEL_COMMAND} needs the interactive TUI"
            )));
        }
        let sid = cx.session_id().to_owned();
        let models = Self::models();
        if models.is_empty() {
            let _ = ui::notify(
                "No models available. Use /login to add a provider.",
                "warning",
            );
            return Ok(());
        }
        let (p, _) = self.preset();
        let current = control::model().map(|m| {
            format!(
                "{}/{}",
                m.get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                m.get("id").and_then(Value::as_str).unwrap_or_default()
            )
        });
        let mut active = self
            .sessions
            .with(&sid, |s| s.active.clone())
            .or_else(|| p.active.clone());
        if let Some(cur) = &current {
            let stale = match &active {
                None => true,
                Some(Active::Single { model }) => model != cur,
                Some(Active::Fusion { lead, .. }) => lead != cur,
            };
            if stale {
                active = Some(Active::Single { model: cur.clone() });
            }
        }
        let fallback_effort = control::thinking_level();
        let keys = picker_rows(&models, &p, active.as_ref());
        let fusion_label = match (&active, &p.default) {
            (Some(Active::Fusion { lead, sidekick, .. }), _)
            | (
                _,
                Pair {
                    lead: Some(lead),
                    sidekick: Some(sidekick),
                },
            ) => format!(
                "Fusion · {} ◆ {}",
                Self::name_of(&models, lead),
                Self::name_of(&models, sidekick)
            ),
            _ => "Fusion · pick a lead and a sidekick…".into(),
        };
        let mark = |k: &str| {
            if current.as_deref() == Some(k) && !matches!(active, Some(Active::Fusion { .. })) {
                " ✓"
            } else {
                ""
            }
        };
        let mut labels = vec![format!(
            "{fusion_label}{}",
            if matches!(active, Some(Active::Fusion { .. })) {
                " ✓"
            } else {
                ""
            }
        )];
        labels.extend(keys.iter().map(|k| {
            let effort = p
                .effort
                .get(k)
                .cloned()
                .unwrap_or_else(|| fallback_effort.clone());
            let badge = p
                .badges
                .get(k)
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default();
            format!(
                "{}{badge} · {}{}",
                Self::name_of(&models, k),
                preset::effort_label(&effort),
                mark(k)
            )
        }));
        let mut details = vec![
            json!({"label": "Fusion", "description": "Lead plans and reviews; a persistent sidekick does the hands-on work"}),
        ];
        details.extend(keys.iter().map(|k| json!({"label": k, "description": price_text(p.prices.get(k).or_else(|| models.iter().find(|m| &m.key == k).and_then(|m| m.cost.as_ref())))})));
        let Some(choice) = ui::dialog(&json!({"method": "select", "title": "/fusion — type to search", "options": labels, "optionDetails": details})).await?.get("value").and_then(Value::as_str).map(ToOwned::to_owned) else {
            return Ok(());
        };
        let mut effort_map = p.effort.clone();
        let sel = if choice == labels[0] {
            let (def_lead, def_side, def_lead_effort, def_side_effort) = match &active {
                Some(Active::Fusion {
                    lead,
                    sidekick,
                    lead_effort,
                    sidekick_effort,
                }) => (
                    Some(lead.clone()),
                    Some(sidekick.clone()),
                    lead_effort.clone(),
                    sidekick_effort.clone(),
                ),
                _ => (
                    p.default.lead.clone(),
                    p.default.sidekick.clone(),
                    None,
                    None,
                ),
            };
            let all: Vec<String> = models.iter().map(|m| m.key.clone()).collect();
            let leads: Vec<String> =
                p.lead
                    .iter()
                    .cloned()
                    .chain(all.iter().cloned())
                    .fold(Vec::new(), |mut v, k| {
                        if !v.contains(&k) {
                            v.push(k);
                        }
                        v
                    });
            let sides: Vec<String> = p.sidekick.iter().cloned().chain(all.iter().cloned()).fold(
                Vec::new(),
                |mut v, k| {
                    if !v.contains(&k) {
                        v.push(k);
                    }
                    v
                },
            );
            let Some(lead) =
                Self::pick_key("Fusion — lead", &leads, &models, def_lead.as_deref()).await
            else {
                return Ok(());
            };
            let Some(sidekick) =
                Self::pick_key("Fusion — sidekick", &sides, &models, def_side.as_deref()).await
            else {
                return Ok(());
            };
            let Some(le) = Self::pick_effort(
                "Lead effort",
                def_lead_effort.as_deref().unwrap_or(&fallback_effort),
            )
            .await
            else {
                return Ok(());
            };
            let Some(se) = Self::pick_effort(
                "Sidekick effort",
                def_side_effort.as_deref().unwrap_or("medium"),
            )
            .await
            else {
                return Ok(());
            };
            Active::Fusion {
                lead,
                sidekick,
                lead_effort: Some(le),
                sidekick_effort: Some(se),
            }
        } else {
            let Some(i) = labels.iter().position(|l| *l == choice) else {
                return Ok(());
            };
            let key = keys[i - 1].clone();
            let cur = p
                .effort
                .get(&key)
                .cloned()
                .unwrap_or_else(|| fallback_effort.clone());
            let Some(e) =
                Self::pick_effort(&format!("{} — effort", Self::name_of(&models, &key)), &cur)
                    .await
            else {
                return Ok(());
            };
            effort_map.insert(key.clone(), e);
            Active::Single { model: key }
        };
        let Ok(Some(confirm)) =
            ui::select("Confirm", &["Confirm", "Confirm and use at startup"]).await
        else {
            return Ok(());
        };
        if confirm == "Confirm and use at startup" {
            self.save_startup(&sel, &effort_map);
        }
        if let Err(e) = self.apply(&sid, &sel, &effort_map, false) {
            let _ = ui::notify(&e, "error");
        }
        self.sync_tools(&cx.cx);
        self.publish_status(&sid);
        Ok(())
    }

    #[allow(
        clippy::unused_self,
        reason = "state lives in files; kept on the extension for one call site style"
    )]
    /// The picker's `Alt+Enter`: remember the startup selection (global preset `startup`).
    fn save_startup(&self, sel: &Active, effort: &std::collections::BTreeMap<String, String>) {
        let path = global_path();
        let mut raw = read_raw(&path).unwrap_or_else(|| json!({}));
        let (model, thinking) = match sel {
            Active::Single { model } => (model.clone(), effort.get(model).cloned()),
            Active::Fusion {
                lead, lead_effort, ..
            } => (lead.clone(), lead_effort.clone()),
        };
        raw["startup"] = json!({"model": model, "thinking": thinking, "active": sel});
        let _ = unipi_kit::paths::write_json(&path, &raw);
        // pi keeps the fresh-session model in `settings.json` (`defaultModel`), so persist the
        // same pair in core settings (unipi-port R4); this does not switch the current session.
        if let Some((provider, id)) = preset::split_key(&model) {
            let _ = control::set_default_model(provider, id, thinking.as_deref());
        }
        let _ = ui::notify(
            &match sel {
                Active::Single { model } => {
                    format!("Default: {model} · {}", thinking.unwrap_or_default())
                }
                Active::Fusion { lead, sidekick, .. } => {
                    format!("Default: Fusion ({lead} + {sidekick})")
                }
            },
            "info",
        );
    }

    async fn preset_editor(&self, cx: &CommandCtx) -> Result<()> {
        if !cx.has_ui() {
            return Err(Error::msg("Edit fusion presets needs the interactive TUI"));
        }
        let models = Self::models();
        let (p, has_project) = self.preset();
        let ms = |title: &str, chosen: &[String]| json!({"method": "multiSelect", "title": title, "options": models.iter().map(|m| json!({"label": format!("{} ({})", m.name, m.key), "value": m.key, "selected": chosen.contains(&m.key)})).collect::<Vec<_>>()});
        let pick = |v: &Value| -> Option<Vec<String>> {
            v.get("value")?.as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
        };
        let Some(lead) = pick(&ui::dialog(&ms("Fusion presets — lead models", &p.lead)).await?)
        else {
            return Ok(());
        };
        let Some(sidekick) =
            pick(&ui::dialog(&ms("Fusion presets — sidekick models", &p.sidekick)).await?)
        else {
            return Ok(());
        };
        let options = if has_project {
            ["This project", "Global"]
        } else {
            ["Global", "This project"]
        };
        let Ok(Some(target)) = ui::select("Save presets to", &options).await else {
            return Ok(());
        };
        let path = self.save_curation(target == "This project", &lead, &sidekick, &p.default);
        let shown = unipi_kit::paths::host_path(&path).unwrap_or(path);
        let _ = ui::notify(
            &format!(
                "Saved preset → {}\n{} lead · {} sidekick",
                shown.display(),
                lead.len(),
                sidekick.len()
            ),
            "info",
        );
        Ok(())
    }

    fn stats(&self, sid: &str) -> String {
        let (active, sk) = self
            .sessions
            .with(sid, |s| (s.active.clone(), s.sidekick.clone()));
        let (Some(Active::Fusion { lead, sidekick, .. }), Some(sk)) = (active, sk) else {
            return "Fusion is not active — pick a Fusion pair with /fusion.".into();
        };
        let models = Self::models();
        let (p, _) = self.preset();
        let cost = |k: &str| {
            p.prices
                .get(k)
                .copied()
                .or_else(|| models.iter().find(|m| m.key == k).and_then(|m| m.cost))
        };
        let (lc, sc) = (cost(&lead), cost(&sidekick));
        let u = *sk.usage.borrow();
        let (s, l, saved) = policy::estimate_savings(&u, lc.as_ref(), sc.as_ref());
        let pricing = if lc.is_none() && sc.is_none() {
            "\nPricing unavailable from provider — set \"prices\" in the fusion preset (preset.json) to estimate savings."
        } else {
            ""
        };
        let reports = sk
            .handoffs
            .borrow()
            .iter()
            .filter(|h| h.done.get().is_some())
            .count();
        format!(
            "Sidekick tokens: in {} · out {} · cached {} · cache write {}\nSidekick cost: ${s:.2} · at lead prices: ${l:.2} · saved: ${saved:.2}\nHandoffs: {reports} · runtime alive: true · busy: {}{pricing}",
            u.input,
            u.output,
            u.cache_read,
            u.cache_write,
            sk.busy()
        )
    }

    async fn session_start(&self, cx: &Ctx) {
        let sid = cx.session_id().to_owned();
        self.stop_runtime(&sid);
        let (p, _) = self.preset();
        let startup = read_raw(&global_path()).and_then(|v| v.get("startup").cloned());
        let settings_model = self
            .settings
            .get(NAMESPACE)
            .get("startup.model")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let mut active = p.active.clone();
        let fresh = cx.raw().kind == oino_extension::wit::types::CallKind::Event
            && session::entries().is_ok_and(|e| e.len() <= 1);
        if fresh {
            if let Some(m) = settings_model {
                active = Some(Active::Single { model: m });
            } else if let Some(a) = startup
                .as_ref()
                .and_then(|s| s.get("active"))
                .and_then(|a| serde_json::from_value(a.clone()).ok())
            {
                active = Some(a);
            }
        }
        self.sessions.with(&sid, |s| {
            *s = State {
                active: active.clone(),
                ..State::default()
            };
        });
        let current = control::model().map(|m| {
            format!(
                "{}/{}",
                m.get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                m.get("id").and_then(Value::as_str).unwrap_or_default()
            )
        });
        let want = match &active {
            Some(Active::Fusion {
                lead, lead_effort, ..
            }) => Some((lead.clone(), lead_effort.clone())),
            Some(Active::Single { model }) if fresh => Some((
                model.clone(),
                self.settings
                    .get(NAMESPACE)
                    .get("startup.thinking")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| p.effort.get(model).cloned()),
            )),
            _ => None,
        };
        if let Some((key, effort)) = want
            && current.as_deref() != Some(key.as_str())
        {
            let ok = preset::split_key(&key)
                .is_some_and(|(pr, id)| control::set_model(pr, id).unwrap_or(false));
            if ok {
                let _ = control::set_thinking_level(effort.as_deref().unwrap_or("medium"));
            } else if matches!(active, Some(Active::Fusion { .. })) {
                self.sessions.with(&sid, |s| s.active = None);
                if cx.has_ui() {
                    let _ = ui::notify(
                        &format!("Fusion lead {key} unavailable — Fusion off"),
                        "warning",
                    );
                }
            }
        }
        self.register_reader(cx).await;
        self.sync_tools(cx);
        self.publish_status(&sid);
    }

    fn model_select(&self, ev: &Event, cx: &Ctx) {
        let sid = cx.session_id();
        let key = ev
            .get("model")
            .map(|m| {
                format!(
                    "{}/{}",
                    m.get("provider")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    m.get("id").and_then(Value::as_str).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let leave = self.sessions.with(
            sid,
            |s| matches!(&s.active, Some(Active::Fusion { lead, .. }) if *lead != key),
        );
        if leave {
            self.stop_runtime(sid);
            let a = Active::Single { model: key };
            self.sessions.with(sid, |s| s.active = Some(a.clone()));
            let (p, _) = self.preset();
            self.save_runtime(&p.effort, &p.recent, Some(&a));
            self.sync_tools(cx);
            self.publish_status(sid);
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per rendered type")]
    fn render(req: &Value) -> Option<Value> {
        let key = req.get("key").and_then(Value::as_str)?;
        let data = req.get("data")?;
        let expanded = req
            .get("expanded")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let line = |glyph: &str, tone: &str, label: &str, meta: String| json!({"t": "text", "value": [{"text": format!("{glyph} "), "tone": tone}, {"text": label, "bold": true, "tone": "tool"}, {"text": meta, "dim": true}]});
        match key {
            "sidekick-step" => {
                let d = data.get("data")?;
                let body = match d.get("kind").and_then(Value::as_str) {
                    Some("tool") => {
                        let mut c = vec![line(
                            "●",
                            "accent",
                            d.get("name").and_then(Value::as_str).unwrap_or("tool"),
                            format!(
                                " {}",
                                d.get("arg").and_then(Value::as_str).unwrap_or_default()
                            ),
                        )];
                        if expanded
                            && let Some(o) = d
                                .get("output")
                                .and_then(Value::as_str)
                                .filter(|o| !o.is_empty())
                        {
                            c.push(json!({"t": "log", "lines": o.split('\n').collect::<Vec<_>>()}));
                        }
                        json!({"t": "stack", "direction": "vertical", "gap": 0, "children": c})
                    }
                    _ => {
                        json!({"t": "markdown", "value": d.get("text").and_then(Value::as_str).unwrap_or_default()})
                    }
                };
                Some(
                    json!({"t": "card", "title": d.get("label").and_then(Value::as_str).unwrap_or("Sidekick"), "rail": "tool", "body": body}),
                )
            }
            "sidekick-completion" => {
                let r: Option<Report> = data
                    .get("details")
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let failed = r
                    .as_ref()
                    .is_some_and(|r| r.status != child::ReportStatus::Completed);
                let meta = r
                    .as_ref()
                    .map(|r| format!(" · {} steps · {}", r.tool_calls, duration(r.duration_ms)))
                    .unwrap_or_default();
                Some(
                    json!({"t": "text", "value": [{"text": if failed { "✗ " } else { "◆ " }, "tone": if failed { "error" } else { "accent" }}, {"text": format!("{}{meta}", if failed { "sidekick failed" } else { "sidekick finished" }), "dim": true}]}),
                )
            }
            "sidekick" => {
                let res = data.get("result").or_else(|| data.get("partialResult"))?;
                let d = res.get("details").cloned().unwrap_or_default();
                if d.get("background").and_then(Value::as_bool) == Some(true) {
                    return Some(line(
                        "◆",
                        "accent",
                        "sidekick",
                        " · continuing in background".into(),
                    ));
                }
                if let Some(p) = d.get("progress") {
                    let started = p.get("startedAt").and_then(Value::as_i64).unwrap_or(0);
                    return Some(line(
                        "◆",
                        "accent",
                        "sidekick",
                        format!(
                            " working · {} steps · {}",
                            p.get("toolCalls").and_then(Value::as_u64).unwrap_or(0),
                            duration(timers::now_ms() - started)
                        ),
                    ));
                }
                let failed = res.get("isError").and_then(Value::as_bool) == Some(true)
                    || d.get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|s| s != "completed");
                let meta = d
                    .get("durationMs")
                    .and_then(Value::as_i64)
                    .map(|ms| {
                        format!(
                            " · {} steps · {} · in {} / out {} tokens",
                            d.get("toolCalls").and_then(Value::as_u64).unwrap_or(0),
                            duration(ms),
                            d.pointer("/usage/input")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            d.pointer("/usage/output")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                        )
                    })
                    .unwrap_or_default();
                Some(line(
                    if failed { "✗" } else { "◆" },
                    if failed { "error" } else { "accent" },
                    if failed {
                        "sidekick failed"
                    } else {
                        "sidekick done"
                    },
                    meta,
                ))
            }
            _ => None,
        }
    }
}

enum Waited {
    Report(Report),
    Aborted,
    Interrupted(String),
    Timeout,
}

/// `sidekick` parameters.
#[must_use]
pub fn sidekick_schema() -> Value {
    json!({"type": "object", "properties": {
        "message": {"type": "string", "description": "A concrete implementation or verification brief for the sidekick"},
        "block": {"type": "boolean", "description": "Wait for completion (default true)"},
    }, "required": ["message"]})
}

impl Extension for Fusion {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        if cx.ctx().child_role() == Some("fusion") {
            return Ok(());
        }
        let ns = namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        cx.tool(
            Tool::<Self>::new("sidekick")
                .label("Sidekick")
                .description(SIDEKICK_DESCRIPTION)
                .parameters(&sidekick_schema())
                .kind(ToolKind::Other)
                .inactive()
                .run(|ext, args: Value, cx| async move { ext.sidekick_tool(&args, &cx).await }),
        );
        for (t, k) in [
            (RendererTarget::CustomEntry, "sidekick-step"),
            (RendererTarget::CustomMessage, "sidekick-completion"),
            (RendererTarget::Tool, "sidekick"),
        ] {
            cx.renderer(t, k, |_e, req, _cx| Self::render(&req));
        }
        cx.command(Command::<Self>::new(MODEL_COMMAND).description("Pick a model or a Fusion lead+sidekick pair (Devin-style picker over your preset)")
            .run(|ext, _a, cx| async move { ext.picker(&cx).await }));
        cx.command(
            Command::<Self>::new("fusion-stats")
                .description("Estimated Fusion savings (sidekick tokens priced at lead rates)")
                .run(|ext, _a, cx| async move { ui::notify(&ext.stats(cx.session_id()), "info") }),
        );
        cx.command(
            Command::<Self>::new(PRESET_COMMAND)
                .description("Edit fusion presets")
                .hidden()
                .run(|ext, _a, cx| async move { ext.preset_editor(&cx).await }),
        );
        cx.input_completer(|_e, req, _cx| async move {
            boost_completion(req.get("text").and_then(Value::as_str).unwrap_or_default())
        });
        cx.service(READER_SERVICE, |ext, p, _caller, cx| async move {
            Ok(ext.reader(&p, &cx).await)
        });
        cx.service(point::FUSION_STATUS, |ext, _p, _c, cx| async move {
            Ok(json!({"active": ext.sessions.with(cx.session_id(), |s| s.active.clone()), "status": unipi_kit::bus::get_raw(topics::FUSION_STATUS)}))
        });
        cx.service(point::FUSION_SET_PAIR, |ext, p, _c, cx| async move {
            let sel: Active =
                serde_json::from_value(p.get("active").cloned().unwrap_or(p.clone()))?;
            let (preset, _) = ext.preset();
            let r = ext.apply(cx.session_id(), &sel, &preset.effort, true);
            ext.sync_tools(&cx);
            ext.publish_status(cx.session_id());
            Ok(match r {
                Ok(()) => json!({"ok": true}),
                Err(m) => json!({"ok": false, "message": m}),
            })
        });
        cx.wait_source(|ext, cx| async move {
            (!cx.is_child()
                && ext
                    .sessions
                    .with(cx.session_id(), |s| !s.background.is_empty()))
            .then(|| "sidekick working".to_owned())
        });
        cx.on(EventKind::BeforeAgentStart, |ext, ev, cx| async move {
            Ok(ext.before_agent_start(&ev, &cx))
        });
        cx.on(EventKind::TurnStart, |ext, _ev, cx| async move {
            ext.sessions
                .with(cx.session_id(), |s| s.edit_nudged = false);
            Ok(())
        });
        for k in [EventKind::TurnEnd, EventKind::AgentSettled] {
            cx.on(k, |ext, _ev, cx| async move {
                ext.publish_status(cx.session_id());
                Ok(())
            });
        }
        cx.on(EventKind::ToolResult, |ext, ev, cx| async move {
            Ok(ext.tool_result(&ev, &cx))
        });
        cx.on(EventKind::ModelSelect, |ext, ev, cx| async move {
            ext.model_select(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.session_start(&cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.stop_runtime(cx.session_id());
            ext.sessions.remove(cx.session_id());
            unipi_kit::bus::emit(&None::<FusionStatus>);
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        Ok(())
    }
}

/// Reader service name (served for `unipi.subagents`' `read_subagent`).
pub const READER_SERVICE: &str = "subagent-reader.read";

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Fusion);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_boost_and_texts() {
        let models = models_from(&[
            json!({"provider": "a", "id": "x", "name": "X", "cost": {"input": 1, "output": 2, "cacheRead": 0.1}}),
            json!({"provider": "b", "id": "y"}),
            json!({"provider": "c", "id": "z"}),
        ]);
        assert_eq!(models[1].name, "y");
        assert!(models[1].cost.is_none());
        let p = Preset {
            recent: vec!["c/z".into()],
            lead: vec!["b/y".into(), "gone/q".into()],
            ..Preset::default()
        };
        assert_eq!(
            picker_rows(
                &models,
                &p,
                Some(&Active::Single {
                    model: "a/x".into()
                })
            ),
            ["a/x", "c/z", "b/y"]
        );
        assert!(boost_completion("/mod").is_some());
        assert!(boost_completion("/modelx").is_none());
        assert_eq!(
            boost_completion("/model").unwrap()[0]["items"][0]["value"],
            "/fusion"
        );
        assert_eq!(duration(12_000), "12s");
        assert_eq!(duration(184_000), "3m04s");
        assert_eq!(
            wake_text(3, 12_000),
            "sidekick working · 3 tool calls · 12s — resumes automatically when done"
        );
        assert_eq!(
            price_text(models[0].cost.as_ref()),
            "$1 / $0.1 / $2 per 1M (in / cached / out)"
        );
        let r = Report {
            id: "h1".into(),
            status: child::ReportStatus::Completed,
            text: "ok".into(),
            usage: Usage {
                input: 5,
                output: 6,
                ..Usage::default()
            },
            tool_calls: 2,
            duration_ms: 3000,
            events: vec![],
            error: None,
        };
        assert_eq!(
            report_text(&r),
            "ok\n\n--- sidekick h1 · completed · 2 tool calls · 3s · in 5 / out 6 tokens"
        );
        let mut prog = Progress::default();
        prog.recent_tools.push("read(x)".into());
        assert_eq!(
            progress_text(&prog, 1000),
            "◆ sidekick working · 0 tool calls · 1s\n  read(x)"
        );
        assert_eq!(sidekick_schema()["required"], json!(["message"]));
        assert_eq!(
            namespace().to_json()["sections"][1]["fields"][2]["command"],
            PRESET_COMMAND
        );
    }
}
