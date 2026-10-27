//! `unipi.subagents` — independent child agents (unipi `packages/subagents`, unipi-port §3.11).
//!
//! | unipi | here |
//! |---|---|
//! | `run_subagent {title, task, profile, is_background?, resume?}` (profile list in the schema) | tool; nested agents via host `agents` ([`child`]) |
//! | `read_subagent {agent_id?, block?, timeout?}` (core, shared with fusion) | tool here; external owners via service `subagents.register-external` + `subagent-reader.read` |
//! | profiles: built-ins + `~/.unipi/config/agents`, `<ws>/.unipi/config/agents` | built-ins + `/ext-global/profiles`, `.oino/agents` ([`profiles`]) |
//! | `/subagents` (tray tab), `/agents` (profile manager) | commands; tray tab via `work-tray.tab` ([`view`]) |
//! | `Ctrl+B` (all foreground runs → background) | shortcut `unipi.subagents.foreground` |
//! | `subagent-step` entries, `subagent-completion` messages, cards | entries/messages + renderers |
//! | prompt section `subagents` | `before_agent_start` `systemPromptOptions.sections.subagents` |
//! | wait source `subagent working` | `wait-source` |
//! | `index.json` per lead session | `/ext-workspace/<wsid>/sessions/<sid>/index.json` |
//! | graph-run port, shared list/manager/transcript | services `subagents.graph-run`, `subagents.profiles`, `subagents.list`, `subagents.transcript`, `subagents.cancel`, `subagents.foreground`; sticky `unipi:subagents:list`; bus `unipi:subagents:run-done` |
//! | settings `subagents` | namespace |

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

pub mod child;
pub mod profiles;
pub mod record;
pub mod view;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use oino_extension::agents::Agent;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::services::{OpenTray, fanout as fan, point};
use unipi_kit::settings::{Field, Kind, Namespace, Section, Store, choice, plain};
use unipi_kit::tray::{Action, ActionReply, TabRequest, TrayTab};
use unipi_kit::{suite, topics};

use crate::child::{Delivery, Done, Progress, Report, Spec, Usage};
use crate::profiles::{AgentFile, Profile, Source};
use crate::record::{CancelledBy, Record, Status};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.subagents";
/// Settings namespace.
pub const NAMESPACE: &str = "subagents";
/// Agents manager command.
pub const AGENTS_COMMAND: &str = "agents";
/// Service an external `read_subagent` owner (fusion) serves.
pub const READER_SERVICE: &str = "subagent-reader.read";
/// Profile list service (graph).
pub const PROFILES_SERVICE: &str = "subagents.profiles";
const AUTO_MODEL: &str = "auto (Fusion sidekick → your model)";
const FG_KEY: &str = "subagents-foreground";
const NEW_LABEL: &str = "＋ New agent…";

struct Run {
    agent: Rc<Agent>,
    progress: RefCell<Progress>,
    done: Rc<Done<Report>>,
    aborted: Cell<bool>,
    graph: bool,
}

#[derive(Default)]
struct Pane {
    view: Option<String>,
    full_output: bool,
    flash: Option<String>,
}

#[derive(Default)]
struct SessionState {
    records: Vec<Record>,
    runs: HashMap<String, Rc<Run>>,
    fg_waits: HashMap<String, Rc<Cell<bool>>>,
    watched: HashSet<String>,
    pane: Pane,
    profiles: Vec<Profile>,
    warned: HashSet<String>,
    externals: Vec<String>,
    tray_open: bool,
}

/// The extension.
#[derive(Default)]
pub struct Subagents {
    settings: Store,
    sessions: SessionMap<SessionState>,
    delivery: Delivery,
}

/// Resolved `subagents` settings (unipi `loadConfig`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Enabled.
    pub enabled: bool,
    /// Default subagent model.
    pub default_model: Option<String>,
    /// Default thinking (`None` = inherit).
    pub default_thinking: Option<String>,
    /// Concurrency cap (1..=16).
    pub max_concurrent: usize,
}

impl Config {
    /// From namespace values.
    #[must_use]
    pub fn from(v: &unipi_kit::settings::Values) -> Self {
        let n = v
            .get("maxConcurrent")
            .and_then(Value::as_f64)
            .unwrap_or(8.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped"
        )]
        let max = if n.is_finite() && n >= 1.0 {
            (n.floor() as usize).min(record::MAX_CONCURRENT_CAP)
        } else {
            record::MAX_CONCURRENT
        };
        let text = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        };
        Self {
            enabled: v.get("enabled").and_then(Value::as_bool) != Some(false),
            default_model: text("defaultModel"),
            default_thinking: text("defaultThinking").filter(|t| t != "inherit"),
            max_concurrent: max,
        }
    }
}

/// Settings schema (unipi `registerSettings({namespace: "subagents"})`).
#[must_use]
pub fn namespace() -> Namespace {
    let thinking: Vec<_> = std::iter::once(choice("inherit", "inherit", "use your thinking level"))
        .chain(
            ["off", "minimal", "low", "medium", "high", "xhigh"]
                .into_iter()
                .map(plain),
        )
        .collect();
    Namespace::new(NAMESPACE, "Subagents", EXTENSION_ID)
        .defaults(json!({"enabled": true, "defaultThinking": "inherit", "maxConcurrent": record::MAX_CONCURRENT}))
        .section(
            Section::new("subagents", "Subagents")
                .field(Field::boolean("enabled", "Enabled").description("Offer run_subagent / read_subagent to the agent. Applies from the next turn."))
                .field(
                    Field::model("defaultModel", "Default subagent model")
                        .description("Model for subagent_explore and custom agents without a model: line. subagent_general always runs on your model.")
                        .with_kind(|k| {
                            if let Kind::Model { filter, empty_label, empty_option, .. } = k {
                                *filter = Some(json!({"capability": "text"}));
                                *empty_label = Some(AUTO_MODEL.into());
                                *empty_option = Some(AUTO_MODEL.into());
                            }
                        }),
                )
                .field(Field::enumeration("defaultThinking", "Default thinking level", thinking).description("For subagent_explore and custom agents without thinking:. subagent_general uses yours."))
                .field(Field::number("maxConcurrent", "Max running at once").with_kind(|k| {
                    if let Kind::Number { min, max, .. } = k {
                        *min = Some(1.0);
                        *max = Some(16.0);
                    }
                }).description("Further run_subagent calls are refused until one finishes.")),
        )
        .section(Section::new("agents", "Agents").field(
            Field::action("manageAgents", "Manage agents…", AGENTS_COMMAND).description("List, create, edit, copy or delete custom agents (global or this project)."),
        ))
}

/// unipi `resolveSubagentModel`: profile → (general: parent) → default → Fusion sidekick → parent.
#[must_use]
pub fn resolve_model(
    profile: &Profile,
    parent: &str,
    config: &Config,
    sidekick: Option<&str>,
) -> String {
    if let Some(m) = &profile.model {
        return m.clone();
    }
    if profile.id == "subagent_general" {
        return parent.to_owned();
    }
    config
        .default_model
        .clone()
        .or_else(|| sidekick.map(ToOwned::to_owned))
        .unwrap_or_else(|| parent.to_owned())
}

/// unipi `resolveSubagentThinking`.
#[must_use]
pub fn resolve_thinking(profile: &Profile, parent: &str, config: &Config) -> String {
    let own = if profile.id == "subagent_general" {
        None
    } else {
        config.default_thinking.clone()
    };
    profile.thinking.clone().or(own).unwrap_or_else(|| {
        if parent.is_empty() {
            "medium".into()
        } else {
            parent.to_owned()
        }
    })
}

/// unipi `resolveModel` fuzzy match over `provider/id` keys (exact, then id/full contains, then name).
#[must_use]
pub fn fuzzy_model(input: &str, models: &[(String, String, String)]) -> Option<String> {
    let q = input.to_lowercase();
    if let Some((p, i, _)) = models
        .iter()
        .find(|(p, i, _)| format!("{p}/{i}").to_lowercase() == q)
    {
        return Some(format!("{p}/{i}"));
    }
    let mut best: Option<(f64, String)> = None;
    for (p, i, name) in models {
        let id = i.to_lowercase();
        let full = format!("{p}/{i}").to_lowercase();
        let nm = name.to_lowercase();
        #[allow(clippy::cast_precision_loss, reason = "score")]
        let score = if id == q || full == q {
            100.0
        } else if id.contains(&q) || full.contains(&q) {
            60.0 + (q.len() as f64 / id.len().max(1) as f64) * 30.0
        } else if nm.contains(&q) {
            40.0 + (q.len() as f64 / nm.len().max(1) as f64) * 20.0
        } else {
            0.0
        };
        if score > best.as_ref().map_or(0.0, |b| b.0) {
            best = Some((score, format!("{p}/{i}")));
        }
    }
    best.filter(|(s, _)| *s >= 20.0).map(|(_, k)| k)
}

fn text_result(text: impl Into<String>, details: Value, is_error: bool) -> ToolResult {
    let mut r = ToolResult::text(text).with_details(details);
    r.is_error = is_error;
    r
}

fn new_agent_id() -> String {
    unipi_kit::fmt::random_hex(8)
}

fn parent_model() -> String {
    control::model()
        .map(|m| {
            format!(
                "{}/{}",
                m.get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                m.get("id").and_then(Value::as_str).unwrap_or_default()
            )
        })
        .unwrap_or_default()
}

fn sidekick_key() -> Option<String> {
    unipi_kit::bus::get_raw(topics::FUSION_STATUS).and_then(|v| {
        v.get("sidekickKey")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

/// Reads the agent files of one directory (`<name>.md` or `<name>/AGENT.md` …).
fn agent_files(dir: &Path) -> Vec<AgentFile> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if path.is_file()
            && let Some(stem) = name.strip_suffix(".md")
        {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                out.push((name.clone(), stem.to_owned(), raw));
            }
        } else if path.is_dir() {
            for cand in profiles::DIR_CANDIDATES {
                if let Ok(raw) = std::fs::read_to_string(path.join(cand)) {
                    out.push((format!("{name}/{cand}"), name.clone(), raw));
                    break;
                }
            }
        }
    }
    out
}

fn agents_dir(source: Source) -> PathBuf {
    match source {
        Source::Project => PathBuf::from("/project/.oino/agents"),
        _ => unipi_kit::paths::global("profiles"),
    }
}

fn load_profiles() -> (Vec<Profile>, Vec<String>) {
    profiles::merge(
        &agent_files(&agents_dir(Source::Global)),
        &agent_files(&agents_dir(Source::Project)),
    )
}

/// Guest dir of the run index and child sessions of `sid`.
fn session_dir(sid: &str) -> PathBuf {
    unipi_kit::paths::workspace(&format!("sessions/{}", unipi_kit::paths::sanitize(sid)))
}

impl Subagents {
    fn config(&self) -> Config {
        Config::from(&self.settings.get(NAMESPACE))
    }

    fn persist(&self, sid: &str) {
        let records = self.sessions.with(sid, |s| s.records.clone());
        let _ = unipi_kit::paths::write_json(&session_dir(sid).join("index.json"), &records);
    }

    fn publish(&self, sid: &str) {
        let records = self.sessions.with(sid, |s| s.records.clone());
        unipi_kit::bus::emit_raw(topics::SUBAGENTS_LIST, &json!({"records": records}));
        unipi_kit::bus::emit_raw(topics::WORK_TRAY_CHANGED, &json!({"tab": view::TAB}));
    }

    fn record(&self, sid: &str, id: &str) -> Option<Record> {
        self.sessions
            .with(sid, |s| s.records.iter().find(|r| r.id == id).cloned())
    }

    fn update(&self, sid: &str, id: &str, f: impl FnOnce(&mut Record)) {
        self.sessions.with(sid, |s| {
            if let Some(r) = s.records.iter_mut().find(|r| r.id == id) {
                f(r);
                r.last_activity = timers::now_ms();
            }
        });
    }

    fn run(&self, sid: &str, id: &str) -> Option<Rc<Run>> {
        self.sessions.with(sid, |s| s.runs.get(id).cloned())
    }

    fn live_tool_calls(&self, sid: &str, rec: &Record) -> u32 {
        self.run(sid, &rec.id)
            .map_or(rec.tool_calls, |r| r.progress.borrow().tool_calls)
    }

    fn live_usage(&self, sid: &str, rec: &Record) -> Option<Usage> {
        self.run(sid, &rec.id)
            .map(|r| r.progress.borrow().usage)
            .or(rec.usage)
    }

    /// Re-reads settings-driven state and agent files; syncs the active tools.
    fn refresh(&self, cx: &Ctx) {
        let (profiles, warnings) = load_profiles();
        let fresh: Vec<String> = self.sessions.with(cx.session_id(), |s| {
            s.profiles = profiles;
            warnings
                .into_iter()
                .filter(|w| s.warned.insert(w.clone()))
                .collect()
        });
        for w in fresh {
            let _ = ui::notify(&format!("subagents: {w}"), "warning");
        }
        let cfg = self.config();
        let want_run = cfg.enabled && !cx.is_child();
        let externals = self
            .sessions
            .with(cx.session_id(), |s| !s.externals.is_empty());
        let want_read = cfg.enabled || externals;
        let mut tools = control::active_tools();
        let before = tools.clone();
        for (name, want) in [("run_subagent", want_run), ("read_subagent", want_read)] {
            let has = tools.iter().any(|t| t == name);
            if want && !has {
                tools.push(name.to_owned());
            } else if !want && has {
                tools.retain(|t| t != name);
            }
        }
        if tools != before {
            let _ = control::set_active_tools(&tools);
        }
    }

    fn restore(&self, sid: &str) {
        let list: Vec<Record> =
            unipi_kit::paths::read_json(&session_dir(sid).join("index.json")).unwrap_or_default();
        self.sessions
            .with(sid, |s| s.records = record::restore(list));
        self.persist(sid);
        self.publish(sid);
    }

    // ── starting and driving runs ──────────────────────────────────────────

    #[allow(clippy::too_many_lines, reason = "unipi manager.start")]
    #[allow(clippy::too_many_arguments, reason = "unipi manager.start options")]
    async fn start(
        &'static self,
        cx: &Ctx,
        title: &str,
        task: &str,
        profile: &Profile,
        background: bool,
        resume: Option<&str>,
        graph: bool,
    ) -> std::result::Result<(Record, Rc<Run>), String> {
        let sid = cx.session_id().to_owned();
        let cfg = self.config();
        let (running, resume_running, prev) = self.sessions.with(&sid, |s| {
            (
                s.runs.len(),
                resume.is_some_and(|r| s.runs.contains_key(r)),
                resume.and_then(|r| s.records.iter().find(|x| x.id == r).cloned()),
            )
        });
        record::admit(
            running,
            cfg.max_concurrent,
            resume,
            resume_running,
            prev.is_some(),
        )?;
        let model = resolve_model(profile, &parent_model(), &cfg, sidekick_key().as_deref());
        let thinking = resolve_thinking(profile, &control::thinking_level(), &cfg);
        let id = resume.map_or_else(new_agent_id, ToOwned::to_owned);
        let dir = session_dir(&sid);
        let _ = std::fs::create_dir_all(&dir);
        let host_dir = unipi_kit::paths::host_path(&dir).map(|p| p.to_string_lossy().into_owned());
        let depth = prev.as_ref().map_or(1, |p| p.depth);
        let spec = Spec {
            label: format!(
                "Subagent \"{}\"",
                if title.is_empty() {
                    prev.as_ref().map_or(id.as_str(), |p| p.title.as_str())
                } else {
                    title
                }
            ),
            model: (!model.is_empty()).then(|| model.clone()),
            thinking: Some(thinking.clone()),
            append_system_prompt: Some(profile.system_prompt.clone()),
            allow: profile.tools.clone(),
            deny: if profile.tools.is_some() {
                Vec::new()
            } else {
                profiles::EXCLUDED_NESTING_TOOLS
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            },
            seed_session_file: prev
                .as_ref()
                .map(|p| p.session_file.clone())
                .filter(|f| !f.is_empty()),
            session_dir: host_dir,
            child_role: "subagent".into(),
            forward_ui: !background,
            max_depth: depth
                .saturating_add(profile.max_nesting.unwrap_or(0))
                .max(1),
        };
        let agent = Rc::new(
            Agent::spawn(&spec.to_json())
                .await
                .map_err(|e| e.to_string())?,
        );
        let now = timers::now_ms();
        let rec = Record {
            id: id.clone(),
            title: if title.is_empty() {
                prev.as_ref()
                    .map_or_else(|| id.clone(), |p| p.title.clone())
            } else {
                title.to_owned()
            },
            profile: profile.id.clone(),
            model,
            thinking: Some(thinking),
            status: Status::Running,
            background,
            started_at: now,
            ended_at: None,
            tool_calls: 0,
            last_activity: now,
            task: Some(record::clip_task(task)),
            report: None,
            error: None,
            cancelled_by: None,
            usage: None,
            session_file: agent
                .session_file()
                .or_else(|| prev.map(|p| p.session_file))
                .unwrap_or_default(),
            depth,
        };
        let run = Rc::new(Run {
            agent,
            progress: RefCell::new(Progress::default()),
            done: Done::new(),
            aborted: Cell::new(false),
            graph,
        });
        self.sessions.with(&sid, |s| {
            s.runs.insert(id.clone(), Rc::clone(&run));
            s.records.retain(|r| r.id != id);
            s.records.push(rec.clone());
        });
        self.persist(&sid);
        self.publish(&sid);
        let driver = self.drive(sid, rec.clone(), Rc::clone(&run), task.to_owned());
        spawn(driver);
        Ok((rec, run))
    }

    /// Streams the child's events until it settles, then settles the record and delivers.
    async fn drive(&'static self, sid: String, rec: Record, run: Rc<Run>, task: String) {
        let mut events = run.agent.events();
        let mut error = run.agent.prompt(&task).await.err().map(|e| e.to_string());
        let mut had_bg = false;
        let mut last_publish = 0;
        while error.is_none() {
            let Some(ev) = events.next().await else {
                error = Some("Subagent session ended".into());
                break;
            };
            let now = timers::now_ms();
            let (steps, finishable, calls) = {
                let mut p = run.progress.borrow_mut();
                let steps = p.apply(&ev, now);
                had_bg |= p.open_bg_tasks > 0;
                (steps, p.finishable(), p.tool_calls)
            };
            for step in steps {
                let mut data = serde_json::to_value(&step).unwrap_or_default();
                data["agentId"] = json!(rec.id);
                data["id"] = json!(rec.id);
                data["title"] = json!(rec.title);
                data["profile"] = json!(rec.profile);
                data["group"] = json!(format!("subagent:{}:{}", rec.id, rec.started_at));
                data["label"] = json!(format!(
                    "{} subagent · {}",
                    record::profile_label(&rec.profile),
                    rec.title
                ));
                let _ = session::append_entry("subagent-step", Some(&data));
            }
            self.update(&sid, &rec.id, |r| r.tool_calls = calls);
            if now - last_publish >= 250 {
                last_publish = now;
                self.publish(&sid);
                self.refresh_watched(&sid);
            }
            if finishable {
                if had_bg {
                    timers::sleep(Duration::from_millis(child::SETTLE_GRACE_MS)).await;
                    if !run.progress.borrow().finishable() {
                        continue;
                    }
                }
                break;
            }
        }
        let duration = timers::now_ms() - rec.started_at;
        let text = run.agent.last_assistant_text();
        let report =
            run.progress
                .borrow_mut()
                .report(&rec.id, run.aborted.get(), text, error, duration);
        self.settle(&sid, &rec.id, &run, &report);
        run.done.set(report.clone());
        self.after_done(&sid, &rec.id, &run, &report).await;
    }

    fn settle(&self, sid: &str, id: &str, run: &Rc<Run>, report: &Report) {
        self.sessions.with(sid, |s| {
            if s.runs.get(id).is_some_and(|r| Rc::ptr_eq(r, run)) {
                s.runs.remove(id);
            }
        });
        self.update(sid, id, |r| {
            r.status = record::status_for(report.status, r.cancelled_by);
            r.ended_at = Some(timers::now_ms());
            r.tool_calls = report.tool_calls;
            r.report = Some(report.text.clone());
            r.error.clone_from(&report.error);
            r.usage = Some(report.usage);
        });
        self.persist(sid);
        self.publish(sid);
        self.refresh_watched(sid);
    }

    async fn after_done(&self, sid: &str, id: &str, run: &Rc<Run>, report: &Report) {
        let rec = self.record(sid, id);
        if run.graph {
            let status = rec.as_ref().map_or(Status::Failed, |r| r.status);
            let result = json!({"status": status.as_str(), "text": report.text, "error": report.error, "toolCalls": report.tool_calls, "usage": {"input": report.usage.input, "output": report.usage.output, "cost": report.usage.cost}});
            unipi_kit::bus::emit_raw(
                topics::SUBAGENTS_RUN_DONE,
                &json!({"runId": id, "result": result}),
            );
        } else if self.delivery.should_deliver(id) && rec.as_ref().is_none_or(|r| r.background) {
            self.deliver(rec.as_ref(), report).await;
        }
    }

    /// unipi `deliverCompletion`.
    async fn deliver(&self, rec: Option<&Record>, report: &Report) {
        if rec.and_then(|r| r.cancelled_by) == Some(CancelledBy::Session) {
            return;
        }
        let status = rec.map_or_else(|| record::status_for(report.status, None), |r| r.status);
        let details = json!({
            "owner": "subagents", "id": report.id, "title": rec.map_or(report.id.as_str(), |r| r.title.as_str()),
            "profile": rec.map(|r| r.profile.clone()), "model": rec.map(|r| r.model.clone()), "status": status.as_str(),
            "cancelledBy": rec.and_then(|r| r.cancelled_by), "toolCalls": report.tool_calls, "durationMs": report.duration_ms,
            "usage": report.usage, "report": report.text.chars().take(20_000).collect::<String>(),
        });
        let msg = session::CustomMessage {
            custom_type: "subagent-completion".into(),
            content: json!(record::completion_content(rec, report, status)),
            display: true,
            details: Some(details),
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

    /// Arms the exactly-once completion delivery after a waiter gave up.
    fn detach_delivery(&'static self, sid: &str, id: &str, run: &Rc<Run>) {
        if !self.delivery.detach(id) {
            return;
        }
        if let Some(report) = run.done.get() {
            if self.delivery.should_deliver(id) {
                let rec = self.record(sid, id);
                spawn(async move { self.deliver(rec.as_ref(), &report).await });
            }
            return;
        }
        let (sid, id, done) = (sid.to_owned(), id.to_owned(), Rc::clone(&run.done));
        spawn(async move {
            let report = done.wait().await;
            if self.delivery.should_deliver(&id) {
                let rec = self.record(&sid, &id);
                self.deliver(rec.as_ref(), &report).await;
            }
        });
    }

    fn cancel(&'static self, sid: &str, id: &str, by: CancelledBy) -> bool {
        let Some(run) = self.run(sid, id) else {
            return false;
        };
        self.update(sid, id, |r| r.cancelled_by = Some(by));
        run.aborted.set(true);
        run.agent.abort();
        self.publish(sid);
        let run2 = Rc::clone(&run);
        spawn(async move {
            timers::sleep(Duration::from_millis(u64::from(record::CANCEL_GRACE_MS))).await;
            if run2.done.get().is_none() {
                run2.progress.borrow_mut().settled = true;
                run2.agent.abort();
            }
        });
        true
    }

    fn set_background(&self, sid: &str, id: &str, background: bool) {
        self.update(sid, id, |r| r.background = background);
        self.persist(sid);
        self.publish(sid);
    }

    // ── foreground waits ───────────────────────────────────────────────────

    async fn wait_foreground(
        &self,
        sid: &str,
        run: &Rc<Run>,
        rec: &Record,
        cx: &ToolCtx,
        timeout_ms: i64,
    ) -> Wait {
        let moved = Rc::new(Cell::new(false));
        self.sessions.with(sid, |s| {
            s.fg_waits.insert(rec.id.clone(), Rc::clone(&moved))
        });
        self.delivery.attach(&rec.id);
        self.set_background(sid, &rec.id, false);
        self.sync_working(sid);
        let started = timers::now_ms();
        let outcome = loop {
            if let Some(r) = run.done.get() {
                break Wait::Report(r);
            }
            if cx.is_cancelled() {
                break Wait::Aborted;
            }
            if control::has_pending_messages() {
                break Wait::Interrupted;
            }
            if moved.get() {
                break Wait::Moved;
            }
            if timers::now_ms() - started >= timeout_ms {
                break Wait::Timeout;
            }
            let calls = run.progress.borrow().tool_calls;
            cx.progress(&ToolResult::text(format!("Subagent \"{}\" working · {}", rec.title, record::plural(calls, "tool call")))
                .with_details(json!({"owner": "subagents", "id": rec.id, "title": rec.title, "profile": rec.profile, "model": rec.model, "status": "running", "startedAt": rec.started_at})));
            timers::sleep(Duration::from_millis(400)).await;
        };
        self.sessions.with(sid, |s| s.fg_waits.remove(&rec.id));
        self.sync_working(sid);
        outcome
    }

    fn sync_working(&self, sid: &str) {
        let want = self
            .sessions
            .with(sid, |s| !s.fg_waits.is_empty() || !s.watched.is_empty());
        let _ = ui::fire(
            &json!({"method": "setWorkingMessage", "message": want.then_some(record::WORKING)}),
        );
    }

    fn refresh_watched(&self, sid: &str) {
        let watched: Vec<String> = self.sessions.with(sid, |s| {
            let runs: HashSet<String> = s.runs.keys().cloned().collect();
            s.watched.retain(|id| runs.contains(id));
            s.watched.iter().cloned().collect()
        });
        if watched.is_empty() {
            let _ = ui::fire(&json!({"method": "setWidget", "widgetKey": FG_KEY}));
            return;
        }
        let now = timers::now_ms();
        let mut cards = Vec::new();
        let mut lines = Vec::new();
        for id in &watched {
            let (Some(rec), Some(run)) = (self.record(sid, id), self.run(sid, id)) else {
                continue;
            };
            let p = run.progress.borrow();
            let items = view::items_from_events(&p.events);
            lines.push(format!(
                "{} subagent {} · {}",
                record::profile_label(&rec.profile),
                rec.title,
                record::stat_line(&rec, p.tool_calls, Some(&p.usage), now)
            ));
            cards.push(json!({"t": "card", "title": format!("{} subagent {}", record::profile_label(&rec.profile), rec.title), "body": view::live_card(&rec, &items, p.tool_calls, Some(&p.usage), now)}));
        }
        let _ = ui::fire(
            &json!({"method": "setWidget", "widgetKey": FG_KEY, "widgetLines": lines, "widgetPlacement": "aboveEditor", "widgetSpec": {"t": "stack", "direction": "vertical", "gap": 0, "children": cards}}),
        );
    }

    /// Tray `f`: watch a background run in the foreground.
    fn foreground(&self, sid: &str, id: &str) -> Option<String> {
        if self.run(sid, id).is_none() {
            return Some("Already finished — ask the agent to resume it.".into());
        }
        let already = self.sessions.with(sid, |s| {
            s.fg_waits.contains_key(id) || !s.watched.insert(id.to_owned())
        });
        if !already {
            self.set_background(sid, id, false);
            self.sync_working(sid);
            self.refresh_watched(sid);
        }
        None
    }

    /// Ctrl+B: every foreground run goes to the background.
    fn background_all(&self, sid: &str) -> bool {
        let (waits, watched): (Vec<Rc<Cell<bool>>>, Vec<String>) = self.sessions.with(sid, |s| {
            (
                s.fg_waits.values().cloned().collect(),
                s.watched.drain().collect(),
            )
        });
        for m in &waits {
            m.set(true);
        }
        for id in &watched {
            self.set_background(sid, id, true);
        }
        self.sync_working(sid);
        self.refresh_watched(sid);
        !waits.is_empty() || !watched.is_empty()
    }

    // ── tools ──────────────────────────────────────────────────────────────

    #[allow(
        clippy::too_many_lines,
        reason = "unipi runSubagent branches one to one"
    )]
    async fn run_subagent(&'static self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let sid = cx.session_id().to_owned();
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (title, task, profile_arg) = (s("title"), s("task"), s("profile"));
        let resume = args
            .get("resume")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(ToOwned::to_owned);
        let profile_name = resume
            .as_deref()
            .and_then(|r| self.record(&sid, r))
            .map_or(profile_arg.clone(), |r| r.profile);
        let profiles = self.sessions.with(&sid, |st| st.profiles.clone());
        let Some(profile) = profiles.iter().find(|p| p.id == profile_name).cloned() else {
            return Ok(text_result(
                format!(
                    "Unknown profile \"{profile_arg}\". Available: {}",
                    profiles
                        .iter()
                        .map(|p| p.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                json!({"owner": "subagents", "status": "failed", "title": title}),
                true,
            ));
        };
        let background =
            resume.is_none() && args.get("is_background").and_then(Value::as_bool) == Some(true);
        let (rec, run) = match self
            .start(
                &cx.cx,
                &title,
                &task,
                &profile,
                background,
                resume.as_deref(),
                false,
            )
            .await
        {
            Ok(x) => x,
            Err(e) => {
                return Ok(text_result(
                    e.clone(),
                    json!({"owner": "subagents", "status": "failed", "title": title, "error": e}),
                    true,
                ));
            }
        };
        let id = rec.id.clone();
        if rec.background {
            self.detach_delivery(&sid, &id, &run);
            return Ok(text_result(
                format!(
                    "Subagent {id} started in the background. You will receive a <subagent_completion_notification agent_id=\"{id}\"> when it finishes; use read_subagent to wait."
                ),
                view::details(&rec, Some("started"), Status::Running),
                false,
            ));
        }
        let waited = self
            .wait_foreground(&sid, &run, &rec, cx, record::FOREGROUND_WAIT_MS)
            .await;
        let base = |phase: &str, status: Status| view::details(&rec, Some(phase), status);
        match waited {
            Wait::Report(report) => {
                self.delivery.consume(&id);
                let now = self.record(&sid, &id).unwrap_or_else(|| rec.clone());
                let mut d = base("done", now.status);
                d["toolCalls"] = json!(report.tool_calls);
                d["durationMs"] = json!(report.duration_ms);
                d["usage"] = json!(report.usage);
                d["error"] = json!(report.error);
                d["cancelledBy"] = json!(now.cancelled_by);
                Ok(text_result(
                    record::report_text(&now, &report, now.status),
                    d,
                    now.status != Status::Completed,
                ))
            }
            Wait::Aborted => {
                self.delivery.consume(&id);
                self.cancel(&sid, &id, CancelledBy::User);
                let mut d = base("done", Status::Cancelled);
                d["cancelledBy"] = json!("user");
                d["toolCalls"] = json!(run.progress.borrow().tool_calls);
                d["durationMs"] = json!(timers::now_ms() - rec.started_at);
                Ok(text_result(
                    format!(
                        "The user cancelled subagent \"{}\" ({id}). Don't restart it unless asked; it can be resumed with resume:{id}.",
                        rec.title
                    ),
                    d,
                    true,
                ))
            }
            other => {
                self.detach_delivery(&sid, &id, &run);
                self.set_background(&sid, &id, true);
                let text = match other {
                    Wait::Moved => format!(
                        "The user moved subagent \"{}\" ({id}) to the background so you don't wait for it. It keeps working; you will receive a <subagent_completion_notification agent_id=\"{id}\"> when it finishes. Do not wait for it with read_subagent — continue with other work, or end your turn and let the notification wake you.",
                        rec.title
                    ),
                    Wait::Interrupted => format!(
                        "A user message arrived while subagent \"{}\" ({id}) was working. It continues in the background. Act on the user's message first; its completion notification will arrive when it finishes (or call read_subagent({{agent_id:\"{id}\", block:true}})).",
                        rec.title
                    ),
                    _ => format!(
                        "Subagent {id} is still running in the background; its completion notification will arrive when it finishes."
                    ),
                };
                Ok(text_result(text, base("moved", Status::Running), false))
            }
        }
    }

    #[allow(clippy::too_many_lines, reason = "unipi reader branches one to one")]
    async fn read_subagent(&'static self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let sid = cx.session_id().to_owned();
        let agent_id = args
            .get("agent_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let block = args.get("block").and_then(Value::as_bool) == Some(true);
        let timeout = args
            .get("timeout")
            .and_then(Value::as_f64)
            .unwrap_or(30.0)
            .min(600.0);
        // External owners (fusion sidekick): ask whether they own the id / have a newer handoff.
        let externals = self.sessions.with(&sid, |s| s.externals.clone());
        let own_latest = self
            .sessions
            .with(&sid, |s| s.records.iter().map(|r| r.started_at).max());
        let own = agent_id
            .as_deref()
            .is_some_and(|id| self.record(&sid, id).is_some());
        if !own {
            for ext in &externals {
                let probe = services::call(
                    Some(ext),
                    READER_SERVICE,
                    &json!({"probe": true, "agent_id": agent_id}),
                    Some(Duration::from_secs(2)),
                )
                .await
                .unwrap_or_default();
                let owns = probe.get("owns").and_then(Value::as_bool) == Some(true);
                let newer = agent_id.is_none()
                    && probe
                        .pointer("/latest/startedAt")
                        .and_then(Value::as_i64)
                        .is_some_and(|t| own_latest.is_none_or(|o| t > o));
                if owns || newer {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "≤ 600 s"
                    )]
                    let wait = Duration::from_secs(timeout as u64 + 10);
                    let reply = services::call(
                        Some(ext),
                        READER_SERVICE,
                        &json!({"agent_id": agent_id, "block": block, "timeout": timeout}),
                        Some(wait),
                    )
                    .await?;
                    return Ok(serde_json::from_value(reply)?);
                }
            }
        }
        let id = agent_id.clone().or_else(|| {
            self.sessions.with(&sid, |s| {
                s.records
                    .iter()
                    .max_by_key(|r| r.started_at)
                    .map(|r| r.id.clone())
            })
        });
        let Some(rec) = id.as_deref().and_then(|i| self.record(&sid, i)) else {
            let text = agent_id.map_or_else(
                || "No subagent has run yet.".to_owned(),
                |a| format!("No subagent found for {a}."),
            );
            return Ok(text_result(text, json!({"owner": "subagents"}), true));
        };
        let id = rec.id.clone();
        let base = |status: Status| view::details(&rec, None, status);
        let Some(run) = self
            .run(&sid, &id)
            .filter(|_| rec.status == Status::Running)
        else {
            let mut d = view::details(&rec, Some("done"), rec.status);
            d["toolCalls"] = json!(rec.tool_calls);
            d["cancelledBy"] = json!(rec.cancelled_by);
            return Ok(text_result(
                record::finished_read_text(&rec),
                d,
                rec.status != Status::Completed,
            ));
        };
        if !block {
            return Ok(text_result(
                format!(
                    "Subagent \"{}\" ({id}) is running — {} so far.",
                    rec.title,
                    record::plural(run.progress.borrow().tool_calls, "tool call")
                ),
                base(Status::Running),
                false,
            ));
        }
        let was_background = rec.background;
        #[allow(clippy::cast_possible_truncation, reason = "≤ 600 s")]
        let waited = self
            .wait_foreground(&sid, &run, &rec, cx, (timeout * 1000.0) as i64)
            .await;
        if let Wait::Report(report) = waited {
            self.delivery.consume(&id);
            let now = self.record(&sid, &id).unwrap_or(rec);
            let mut d = view::details(&now, Some("done"), now.status);
            d["toolCalls"] = json!(report.tool_calls);
            d["durationMs"] = json!(report.duration_ms);
            return Ok(text_result(
                record::report_text(&now, &report, now.status),
                d,
                now.status != Status::Completed,
            ));
        }
        self.detach_delivery(&sid, &id, &run);
        if was_background {
            self.set_background(&sid, &id, true);
        }
        let text = match waited {
            Wait::Aborted => {
                format!("Stopped waiting for subagent {id}; it keeps running in the background.")
            }
            Wait::Moved => format!(
                "The user moved subagent {id} to the background; stop waiting and continue — its completion notification will arrive."
            ),
            Wait::Interrupted => format!(
                "A user message arrived while waiting for subagent \"{}\" ({id}); it keeps running. Act on the message first.",
                rec.title
            ),
            _ => format!("Subagent {id} is still running."),
        };
        Ok(text_result(text, base(Status::Running), false))
    }

    // ── tray ───────────────────────────────────────────────────────────────

    fn tab(&self, cx: &Ctx, req: &TabRequest) -> TrayTab {
        let sid = cx.session_id();
        if let Some(id) = &req.initial_id
            && self.record(sid, id).is_some()
        {
            self.sessions.with(sid, |s| s.pane.view = Some(id.clone()));
        }
        self.sessions.with(sid, |s| s.tray_open = req.open);
        let records = self.sessions.with(sid, |s| s.records.clone());
        let counts = unipi_kit::tray::Counts {
            total: u32::try_from(records.len()).unwrap_or(u32::MAX),
            running: u32::try_from(
                records
                    .iter()
                    .filter(|r| r.status == Status::Running)
                    .count(),
            )
            .unwrap_or(u32::MAX),
        };
        TrayTab {
            id: view::TAB.into(),
            label: "Subagents".into(),
            short_label: None,
            order: 1,
            counts,
            visible: true,
            body: req.open.then(|| self.body(sid, &records)),
            owner: None,
        }
    }

    fn transcript(&self, sid: &str, rec: &Record) -> Vec<view::Item> {
        let file = unipi_kit::paths::try_read(&self.guest_session_file(&rec.session_file))
            .unwrap_or_default();
        let events = self
            .run(sid, &rec.id)
            .map(|r| r.progress.borrow().events.clone());
        view::build_transcript(
            view::items_from_session(&file),
            rec.task.as_deref(),
            rec.status == Status::Running,
            events.as_deref(),
        )
    }

    /// Host child-session path → guest path under `/ext-workspace` (index dir).
    fn guest_session_file(&self, host: &str) -> PathBuf {
        let _ = self;
        Path::new(host)
            .file_name()
            .map_or_else(PathBuf::new, |name| {
                let dir = unipi_kit::paths::workspace("sessions");
                std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|d| d.path().join(name))
                    .find(|p| p.exists())
                    .unwrap_or_default()
            })
    }

    fn body(&self, sid: &str, records: &[Record]) -> Value {
        let now = timers::now_ms();
        let (view, full, flash) = self.sessions.with(sid, |s| {
            (
                s.pane.view.clone(),
                s.pane.full_output,
                s.pane.flash.clone(),
            )
        });
        if let Some(rec) = view.and_then(|id| records.iter().find(|r| r.id == id)) {
            let items = self.transcript(sid, rec);
            return view::detail_body(
                rec,
                &items,
                self.live_tool_calls(sid, rec),
                self.live_usage(sid, rec).as_ref(),
                full,
                flash.as_deref(),
                now,
            );
        }
        let rows: Vec<Value> = view::ordered(records)
            .iter()
            .map(|r| {
                view::row(
                    r,
                    self.live_tool_calls(sid, r),
                    self.live_usage(sid, r).as_ref(),
                    now,
                )
            })
            .collect();
        view::list_body(&rows, flash.as_deref())
    }

    fn tray_action(&'static self, cx: &Ctx, action: &Action) -> ActionReply {
        let sid = cx.session_id();
        let (verb, arg) = action
            .action_id
            .split_once(':')
            .map_or((action.action_id.as_str(), None), |(v, a)| {
                (v, Some(a.to_owned()))
            });
        let target = arg.or_else(|| {
            action
                .value
                .as_ref()
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
        self.sessions.with(sid, |s| s.pane.flash = None);
        let mut close = false;
        match (verb, target) {
            ("open", Some(id)) => self.sessions.with(sid, |s| {
                s.pane.view = Some(id);
                s.pane.full_output = false;
            }),
            ("back", _) => self.sessions.with(sid, |s| s.pane.view = None),
            ("output", _) => self
                .sessions
                .with(sid, |s| s.pane.full_output = !s.pane.full_output),
            ("foreground", Some(id)) => match self.foreground(sid, &id) {
                Some(msg) => self.sessions.with(sid, |s| s.pane.flash = Some(msg)),
                None => close = true,
            },
            ("cancel", Some(id)) if !self.cancel(sid, &id, CancelledBy::User) => {
                self.sessions
                    .with(sid, |s| s.pane.flash = Some("Not running.".into()));
            }
            _ => {}
        }
        self.publish(sid);
        ActionReply {
            close,
            refresh: true,
        }
    }

    // ── /agents ──────────────────────────────────────────────────────

    async fn pick_model() -> Option<Option<String>> {
        const DEFAULT: &str = "Default subagent model (recommended)";
        const OTHER: &str = "Other… (type a name — fuzzy match)";
        let mut choices: Vec<(String, String)> = Vec::new();
        let parent = parent_model();
        if parent.len() > 1 {
            choices.push((format!("Your current model — {parent}"), parent));
        }
        if let Some(sk) = sidekick_key() {
            choices.push((format!("Fusion sidekick — {sk}"), sk));
        }
        let mut labels = vec![DEFAULT.to_owned()];
        labels.extend(choices.iter().map(|c| c.0.clone()));
        labels.push(OTHER.to_owned());
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let choice = ui::select("Model", &refs).await.ok()??;
        if choice == DEFAULT {
            return Some(None);
        }
        if choice != OTHER {
            return choices
                .into_iter()
                .find(|c| c.0 == choice)
                .map(|c| Some(c.1));
        }
        let models: Vec<(String, String, String)> = control::models()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let g = |k: &str| {
                    m.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                (g("provider"), g("id"), g("name"))
            })
            .collect();
        loop {
            let raw = ui::input(
                "Model name (e.g. deepseek-flash, sonnet, openrouter/…)",
                Some(""),
            )
            .await
            .ok()??;
            let raw = raw.trim().to_owned();
            if raw.is_empty() {
                return None;
            }
            if models.is_empty() {
                return Some(Some(raw));
            }
            match fuzzy_model(&raw, &models) {
                Some(k) => return Some(Some(k)),
                None => {
                    let _ = ui::notify(&format!("No model matches \"{raw}\"."), "warning");
                }
            }
        }
    }

    async fn pick_tools() -> Option<Option<Vec<String>>> {
        let read = format!("Read-only ({})", profiles::EXPLORE_TOOLS.join(", "));
        let all = "All tools (like subagent_general)";
        let custom = "Custom list…";
        let choice = ui::select("Tools", &[read.as_str(), all, custom])
            .await
            .ok()??;
        if choice == read {
            return Some(Some(
                profiles::EXPLORE_TOOLS
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            ));
        }
        if choice == all {
            return Some(None);
        }
        let raw = ui::input(
            "Tools (comma-separated)",
            Some("read, grep, find, ls, bash"),
        )
        .await
        .ok()??;
        let list: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Some((!list.is_empty()).then_some(list))
    }

    async fn editor(title: &str, prefill: &str) -> Option<String> {
        ui::dialog(&json!({"method": "editor", "title": title, "prefill": prefill}))
            .await
            .ok()?
            .get("value")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    async fn create_agent(existing: &[String]) {
        let name = loop {
            let Ok(Some(raw)) = ui::input("New agent — name (e.g. reviewer)", Some("")).await
            else {
                return;
            };
            match profiles::validate_name(raw.trim(), existing) {
                Some(err) => {
                    let _ = ui::notify(&err, "warning");
                }
                None => break raw.trim().to_owned(),
            }
        };
        let Ok(Some(scope)) = ui::select(
            &format!("Save {name} for"),
            &["This project (.oino/agents)", "All projects (global)"],
        )
        .await
        else {
            return;
        };
        let source = if scope.starts_with("This") {
            Source::Project
        } else {
            Source::Global
        };
        let Ok(Some(description)) = ui::input(
            "Description — the parent agent reads this to pick the profile",
            Some(""),
        )
        .await
        else {
            return;
        };
        let description = description.trim().to_owned();
        if description.is_empty() {
            return;
        }
        let Some(model) = Self::pick_model().await else {
            return;
        };
        let Some(tools) = Self::pick_tools().await else {
            return;
        };
        let Some(prompt) = Self::editor(
            &format!("{name} — system prompt (enter saves)"),
            &profiles::prompt_template(&name, &description),
        )
        .await
        else {
            return;
        };
        if prompt.trim().is_empty() {
            return;
        }
        let file = agents_dir(source).join(format!("{name}.md"));
        let md = profiles::agent_markdown(&profiles::Draft {
            name: name.clone(),
            description,
            model,
            tools,
            thinking: None,
            prompt,
        });
        let saved = unipi_kit::paths::ensure_parent(&file).and_then(|()| std::fs::write(&file, md));
        let shown = unipi_kit::paths::host_path(&file).unwrap_or_else(|| file.clone());
        let _ = match saved {
            Ok(()) => ui::notify(
                &format!(
                    "Agent {name} saved to {}. The agent can use it from the next turn.",
                    shown.display()
                ),
                "info",
            ),
            Err(e) => ui::notify(&format!("Could not save {}: {e}", shown.display()), "error"),
        };
    }

    fn agent_file(p: &Profile) -> Option<PathBuf> {
        if p.source == Source::Builtin {
            return None;
        }
        let dir = agents_dir(p.source);
        std::iter::once(dir.join(format!("{}.md", p.id)))
            .chain(
                profiles::DIR_CANDIDATES
                    .iter()
                    .map(|c| dir.join(&p.id).join(c)),
            )
            .find(|f| f.exists())
    }

    async fn manage_custom(p: &Profile) {
        let Some(file) = Self::agent_file(p) else {
            let _ = ui::notify(&format!("Can't find the file for {}.", p.id), "error");
            return;
        };
        let shown = unipi_kit::paths::host_path(&file)
            .unwrap_or_else(|| file.clone())
            .display()
            .to_string();
        let other = if p.source == Source::Project {
            Source::Global
        } else {
            Source::Project
        };
        let copy = format!(
            "Copy to {}",
            if other == Source::Global {
                "all projects (global)"
            } else {
                "this project"
            }
        );
        let Ok(Some(choice)) = ui::select(
            &format!("{} ({}) — {shown}", p.id, p.source.as_str()),
            &["Edit file", copy.as_str(), "Delete"],
        )
        .await
        else {
            return;
        };
        if choice == "Edit file" {
            let before = std::fs::read_to_string(&file).unwrap_or_default();
            let Some(after) = Self::editor(&format!("Edit {shown}"), &before).await else {
                return;
            };
            if after == before {
                return;
            }
            let _ = match std::fs::write(&file, &after) {
                Ok(()) => ui::notify(&format!("Saved {shown}."), "info"),
                Err(e) => ui::notify(&format!("Not saved — {e}"), "error"),
            };
        } else if choice == copy {
            let target = agents_dir(other).join(format!("{}.md", p.id));
            let tshown = unipi_kit::paths::host_path(&target)
                .unwrap_or_else(|| target.clone())
                .display()
                .to_string();
            if target.exists()
                && !ui::confirm("Overwrite?", &format!("{tshown} already exists."))
                    .await
                    .unwrap_or(false)
            {
                return;
            }
            let ok = unipi_kit::paths::ensure_parent(&target)
                .and_then(|()| std::fs::copy(&file, &target).map(|_| ()));
            let _ = match ok {
                Ok(()) => ui::notify(&format!("Copied to {tshown}."), "info"),
                Err(e) => ui::notify(&format!("Copy failed: {e}"), "error"),
            };
        } else if choice == "Delete"
            && ui::confirm(&format!("Delete {}?", p.id), &shown)
                .await
                .unwrap_or(false)
        {
            let _ = std::fs::remove_file(&file);
            let _ = ui::notify(&format!("Deleted {shown}."), "info");
        }
    }

    async fn agents_manager(&self, cx: &CommandCtx) {
        if !cx.has_ui() {
            return;
        }
        loop {
            let (profiles, warnings) = load_profiles();
            for w in warnings {
                let _ = ui::notify(&format!("subagents: {w}"), "warning");
            }
            let mut labels: Vec<String> = profiles.iter().map(profiles::describe).collect();
            labels.push(NEW_LABEL.into());
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let Ok(Some(choice)) = ui::select(
                "Subagent profiles — pick one to manage, or create a new agent",
                &refs,
            )
            .await
            else {
                break;
            };
            if choice == NEW_LABEL {
                Self::create_agent(&profiles.iter().map(|p| p.id.clone()).collect::<Vec<_>>())
                    .await;
                continue;
            }
            let Some(p) = labels
                .iter()
                .position(|l| *l == choice)
                .and_then(|i| profiles.get(i))
            else {
                break;
            };
            if p.source == Source::Builtin {
                let _ = ui::notify(
                    &format!(
                        "{} is built in and can't be edited. Create a custom agent to change the model, tools or prompt.",
                        p.id
                    ),
                    "info",
                );
                continue;
            }
            Self::manage_custom(p).await;
        }
        self.refresh(&cx.cx);
    }

    // ── rendering ──────────────────────────────────────────────────────────

    fn render(&self, req: &Value, cx: &Ctx) -> Option<Value> {
        let key = req.get("key").and_then(Value::as_str)?;
        let expanded = req
            .get("expanded")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let data = req.get("data")?;
        match key {
            "subagent-completion" => Some(view::completion_card(data.get("details")?, expanded)),
            "subagent-step" => Some(view::step_card(data.get("data")?, expanded)),
            "run_subagent" | "read_subagent" => {
                if let Some(d) = data.pointer("/partialResult/details")
                    && let Some(id) = d.get("id").and_then(Value::as_str)
                    && let (Some(rec), Some(run)) = (
                        self.record(cx.session_id(), id),
                        self.run(cx.session_id(), id),
                    )
                {
                    let p = run.progress.borrow();
                    return Some(view::live_card(
                        &rec,
                        &view::items_from_events(&p.events),
                        p.tool_calls,
                        Some(&p.usage),
                        timers::now_ms(),
                    ));
                }
                let d = data.pointer("/result/details")?;
                let outcome = view::card_outcome(d);
                if !expanded || d.get("phase").and_then(Value::as_str) != Some("done") {
                    return Some(outcome);
                }
                let text = data
                    .pointer("/result/content/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let report = text.split("\n\n--- subagent ").next().unwrap_or_default();
                Some(
                    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": [outcome, {"t": "markdown", "value": report}]}),
                )
            }
            _ => None,
        }
    }

    fn before_agent_start(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        self.refresh(cx);
        let mut opts = ev
            .get("systemPromptOptions")
            .cloned()
            .filter(Value::is_object)?;
        if !opts.get("sections").is_some_and(Value::is_object) {
            opts["sections"] = json!({});
        }
        let sections = opts.get_mut("sections").and_then(Value::as_object_mut)?;
        if self.config().enabled {
            let profiles = self.sessions.with(cx.session_id(), |s| s.profiles.clone());
            let fusion =
                unipi_kit::bus::get_raw(topics::FUSION_STATUS).is_some_and(|v| !v.is_null());
            sections.insert(
                "subagents".into(),
                json!(record::prompt_section(&profiles, fusion)),
            );
        } else {
            sections.remove("subagents");
        }
        Some(json!({"systemPromptOptions": opts}))
    }

    async fn graph_run(&'static self, payload: &Value, cx: &Ctx) -> Value {
        let cfg = self.config();
        if !cfg.enabled || cx.is_child() {
            return json!({"error": "subagents are disabled in this session"});
        }
        let sid = cx.session_id();
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let resume = payload
            .get("resume")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let profiles = self.sessions.with(sid, |st| st.profiles.clone());
        let want = resume
            .as_deref()
            .and_then(|r| self.record(sid, r))
            .map_or_else(|| s("profile"), |r| r.profile);
        let Some(profile) = profiles.iter().find(|p| p.id == want).cloned() else {
            return json!({"error": format!("unknown subagent profile \"{}\" (available: {})", s("profile"), profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", "))});
        };
        match self
            .start(
                cx,
                &s("title"),
                &s("task"),
                &profile,
                true,
                resume.as_deref(),
                true,
            )
            .await
        {
            Ok((rec, _)) => json!({"runId": rec.id}),
            Err(e) => {
                json!({"retryable": e.starts_with("Maximum") || e.contains("still running"), "error": e})
            }
        }
    }

    fn shutdown(&self, sid: &str) {
        let runs: Vec<(String, Rc<Run>)> = self.sessions.with(sid, |s| s.runs.drain().collect());
        let now = timers::now_ms();
        for (id, run) in &runs {
            self.update(sid, id, |r| {
                r.status = Status::Cancelled;
                r.cancelled_by = Some(CancelledBy::Session);
                r.ended_at = Some(now);
                r.error = Some("session ended while it ran".into());
            });
            run.aborted.set(true);
            run.agent.abort();
        }
        self.persist(sid);
        self.publish(sid);
        self.sessions.remove(sid);
    }
}

/// Outcome of a foreground wait.
enum Wait {
    Report(Report),
    Aborted,
    Interrupted,
    Moved,
    Timeout,
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

/// `run_subagent` parameters with the profile list.
#[must_use]
pub fn run_schema(profiles: &[Profile]) -> Value {
    let list = profiles
        .iter()
        .map(|p| format!("{} — {}", p.id, p.description))
        .collect::<Vec<_>>()
        .join("; ");
    json!({"type": "object", "properties": {
        "title": {"type": "string", "description": "Short 3-8 word title shown to the user"},
        "task": {"type": "string", "description": "Complete, self-contained instructions. The subagent does not see this conversation: include the goal, relevant paths, constraints, and what to report back."},
        "profile": {"type": "string", "description": format!("Profile name. Available: {list}. Custom agents added later are listed in the Subagents section of the system prompt.")},
        "is_background": {"type": "boolean", "description": "Run without waiting; report arrives via notification (default false)"},
        "resume": {"type": "string", "description": "agent_id of an earlier subagent to continue (keeps its context); always runs in the foreground"},
    }, "required": ["title", "task", "profile"]})
}

/// `read_subagent` parameters.
#[must_use]
pub fn read_schema() -> Value {
    json!({"type": "object", "properties": {
        "agent_id": {"type": "string", "description": "Subagent or sidekick id; omit for the most recent"},
        "block": {"type": "boolean", "description": "Wait for completion (default false)"},
        "timeout": {"type": "number", "description": "Maximum wait in seconds (default 30, max 600)"},
    }})
}

impl Extension for Subagents {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        let (profiles_now, _) = load_profiles();
        if !cx.ctx().is_child() {
            cx.tool(
                Tool::<Self>::new("run_subagent")
                    .label("Run Subagent")
                    .description(record::RUN_DESCRIPTION)
                    .parameters(&run_schema(&profiles_now))
                    .kind(ToolKind::Other)
                    .run(|ext, args: Value, cx| async move { ext.run_subagent(&args, &cx).await }),
            );
        }
        cx.tool(
            Tool::<Self>::new("read_subagent")
                .label("Read Subagent")
                .description(record::READ_DESCRIPTION)
                .parameters(&read_schema())
                .kind(ToolKind::Read)
                .run(|ext, args: Value, cx| async move { ext.read_subagent(&args, &cx).await }),
        );
        for (target, key) in [
            (RendererTarget::CustomMessage, "subagent-completion"),
            (RendererTarget::CustomEntry, "subagent-step"),
            (RendererTarget::Tool, "run_subagent"),
            (RendererTarget::Tool, "read_subagent"),
        ] {
            cx.renderer(target, key, |ext, req, cx| ext.render(&req, &cx));
        }
        cx.command(
            Command::<Self>::new("subagents")
                .description("Open the Subagents tab of the work tray (also ↓ from an empty input)")
                .run(|ext, _args, cx| async move {
                    if ext.sessions.with(cx.session_id(), |s| s.records.is_empty()) {
                        let _ = ui::notify("No subagents in this session yet.", "info");
                        return Ok(());
                    }
                    let req = serde_json::to_value(OpenTray {
                        tab: Some(view::TAB.into()),
                        initial_id: None,
                    })?;
                    let _ = services::call(
                        Some(suite::pkg::CORE.id),
                        point::CORE_OPEN_TRAY,
                        &req,
                        None,
                    )
                    .await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new(AGENTS_COMMAND)
                .description(
                    "Manage subagent profiles: list, create, edit, copy, delete custom agents",
                )
                .run(|ext, _args, cx| async move {
                    ext.agents_manager(&cx).await;
                    Ok(())
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.subagents.foreground", &["ctrl+b"])
                .description("Move foreground subagents to the background")
                .run(|ext, cx| async move {
                    ext.background_all(cx.session_id());
                    Ok(())
                }),
        );
        cx.service(fan::WORK_TRAY_TAB, |ext, payload, _caller, cx| async move {
            let req: TabRequest = serde_json::from_value(payload).unwrap_or_default();
            Ok(json!([ext.tab(&cx, &req)]))
        });
        cx.service(
            point::WORK_TRAY_ACTION,
            |ext, payload, _caller, cx| async move {
                let action: Action = serde_json::from_value(payload)?;
                Ok(serde_json::to_value(ext.tray_action(&cx, &action))?)
            },
        );
        cx.service(point::SUBAGENTS_LIST, |ext, _p, _caller, cx| async move {
            Ok(json!(
                ext.sessions.with(cx.session_id(), |s| s.records.clone())
            ))
        });
        cx.service(point::SUBAGENTS_TRANSCRIPT, |ext, p, _caller, cx| async move {
            let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
            let Some(rec) = ext.record(cx.session_id(), id) else { return Ok(Value::Null) };
            let items: Vec<Value> = ext.transcript(cx.session_id(), &rec).iter().map(|i| match i {
                view::Item::Task(t) => json!({"kind": "task", "text": t}),
                view::Item::Text(t) => json!({"kind": "text", "text": t}),
                view::Item::Tool { name, arg, output, is_error, running, duration_ms } => json!({"kind": "tool", "name": name, "arg": arg, "output": output, "isError": is_error, "running": running, "durationMs": duration_ms}),
            }).collect();
            Ok(json!({"items": items, "report": rec.report, "status": rec.status}))
        });
        cx.service(point::SUBAGENTS_CANCEL, |ext, p, _caller, cx| async move {
            let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
            Ok(json!({"cancelled": ext.cancel(cx.session_id(), id, CancelledBy::User)}))
        });
        cx.service(
            point::SUBAGENTS_FOREGROUND,
            |ext, p, _caller, cx| async move {
                let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
                Ok(json!({"error": ext.foreground(cx.session_id(), id)}))
            },
        );
        cx.service(
            point::SUBAGENTS_GRAPH_RUN,
            |ext, p, _caller, cx| async move { Ok(ext.graph_run(&p, &cx).await) },
        );
        cx.service(PROFILES_SERVICE, |ext, _p, _caller, cx| async move {
            let on = ext.config().enabled && !cx.is_child();
            Ok(json!(if on {
                ext.sessions.with(cx.session_id(), |s| {
                    s.profiles.iter().map(|p| p.id.clone()).collect::<Vec<_>>()
                })
            } else {
                Vec::new()
            }))
        });
        cx.service(
            point::SUBAGENTS_REGISTER_EXTERNAL,
            |ext, p, caller, cx| async move {
                let owner = p
                    .get("owner")
                    .and_then(Value::as_str)
                    .map_or(caller, ToOwned::to_owned);
                ext.sessions.with(cx.session_id(), |s| {
                    if !s.externals.contains(&owner) {
                        s.externals.push(owner);
                    }
                });
                ext.refresh(&cx);
                Ok(json!({"ok": true}))
            },
        );
        cx.wait_source(|ext, cx| async move {
            if cx.is_child() {
                return None;
            }
            record::background_running_reason(
                &ext.sessions.with(cx.session_id(), |s| s.records.clone()),
            )
        });
        cx.on(EventKind::BeforeAgentStart, |ext, ev, cx| async move {
            Ok(ext.before_agent_start(&ev, &cx))
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.restore(cx.session_id());
            ext.refresh(&cx);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.shutdown(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                ext.refresh(&cx);
            }
            Ok(())
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Subagents);

#[cfg(test)]
mod tests {
    use super::*;
    use unipi_kit::settings::Values;

    #[test]
    fn config_models_and_schemas() {
        let c = Config::from(&Values(
            json!({"maxConcurrent": 40, "defaultThinking": "inherit", "defaultModel": " "}),
        ));
        assert_eq!(
            (c.max_concurrent, c.default_thinking, c.default_model),
            (16, None, None)
        );
        assert_eq!(
            Config::from(&Values(json!({"maxConcurrent": 0}))).max_concurrent,
            8
        );
        let b = profiles::builtins();
        let cfg = Config {
            enabled: true,
            default_model: None,
            default_thinking: Some("low".into()),
            max_concurrent: 8,
        };
        assert_eq!(resolve_model(&b[0], "p/m", &cfg, Some("s/k")), "s/k");
        assert_eq!(resolve_model(&b[1], "p/m", &cfg, Some("s/k")), "p/m");
        assert_eq!(resolve_thinking(&b[0], "high", &cfg), "low");
        assert_eq!(resolve_thinking(&b[1], "high", &cfg), "high");
        let models = vec![
            (
                "anthropic".into(),
                "claude-sonnet-4".into(),
                "Claude Sonnet".into(),
            ),
            ("ds".into(), "deepseek-flash".into(), String::new()),
        ];
        assert_eq!(
            fuzzy_model("sonnet", &models).as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
        assert_eq!(
            fuzzy_model("DS/deepseek-flash", &models).as_deref(),
            Some("ds/deepseek-flash")
        );
        assert_eq!(fuzzy_model("zzz", &models), None);
        let s = run_schema(&b);
        assert!(
            s["properties"]["profile"]["description"]
                .as_str()
                .unwrap()
                .contains("subagent_explore — Read-only")
        );
        assert_eq!(s["required"], json!(["title", "task", "profile"]));
        assert_eq!(read_schema()["properties"]["timeout"]["type"], "number");
        let v = namespace().to_json();
        assert_eq!(v["sections"][1]["fields"][0]["command"], AGENTS_COMMAND);
    }
}
