//! `unipi.dream` — Oino's daily memory consolidation (unipi `packages/dream`, unipi-port
//! §3.20; v3 J2/J3: Devin "Memory and Dreaming" parity on the `MemPalace` backend).
//!
//! A dream is a background agent pass over the recent sessions of **every project** and the
//! memories they left behind. It consolidates overlapping memories (`memory_store` with
//! `merge_from`), removes transient details, adds lessons the sessions missed (failed tool
//! calls + recoveries, corrections, stated preferences), prunes stale memories no session
//! used (per-memory usage from `unipi.memory`'s `memory.inventory`), keeps source session
//! links and explicit preferences, and proposes skills / checks for approval.
//!
//! | unipi | here |
//! |---|---|
//! | due check on open (`minSessions`, `minGapHours`) | a host cron job ([`schedule`]) at `schedule.time` (daily) or every `schedule.intervalHours`; gates applied by `/dream run --scheduled`; an interactive session catches up on an overdue dream when no host scheduler is reachable |
//! | scrubbed struggle digests | [`digest::digest_recent`] over `/agent/sessions/*/*.jsonl` (all projects, since the last dream) |
//! | detached `pi -p` child with craft-skill + memory tools, `timeout maxRuntimeMin` | nested agent (D8): `persistence: File` under the run's staging dir, extensions `only: [unipi.memory]`, aborted at `maxRuntimeMin`; heartbeat in `run.json` so every process sees it; interrupted runs resume once |
//! | report card entry `unipi-dream-report-card` + renderer | custom entry + OUS renderer |
//! | work-tray Dream tab (list/detail, o/s/r/a/x/d/1–9) | `work-tray.tab` / `work-tray.action` ([`tray`]) |
//! | `/dream [status\|tray\|report\|run\|stop\|approve <n>\|reject <n>]` | command (+ `run --scheduled`, `schedule`) |
//! | `unipi.dream.api` | services `dream.status`, `dream.run`, `dream.stop`, `dream.detail`, `dream.runs`, `dream.approve`, `dream.reject`, `dream.dismiss` |
//! | craft-skill `check` before an approved skill lands | [`store::craft_check`] (Rust port); the bundled `craft-skill` skill keeps its scripts |
//! | `dream` settings | [`settings`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod digest;
pub mod schedule;
pub mod settings;
pub mod store;
pub mod tray;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use futures::FutureExt;
use oino_extension::agents::Agent;
use oino_extension::command::CompletionItem;
use oino_extension::host_service;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::ui::widgets::prelude::*;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::services::{fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::topics;
use unipi_kit::tray::{Action, ActionReply, Counts, TabRequest, TrayTab};

use crate::settings::{DreamSettings, EXTENSION_ID, Mode, NAMESPACE};
use crate::store::{DreamState, LastCheck, Lock, Run, RunMeta, Status};
use crate::tray::View;

/// Report card entry type.
pub const REPORT_CARD: &str = "unipi-dream-report-card";
const POLL_TIMER: &str = "dream-poll";
const USAGE: &str = "usage: /dream status|tray|report|run|stop|schedule|approve <n>|reject <n>";
/// Most recent sessions one dream digests.
pub const MAX_SESSIONS: usize = 80;
/// Most memory projects one dream covers.
pub const MAX_PROJECTS: usize = 12;
/// How far back the first dream looks.
pub const FIRST_WINDOW_MS: i64 = 7 * 24 * 3_600_000;
/// How often an interactive session re-checks for an overdue dream.
const AUTO_CHECK_MS: i64 = 30 * 60_000;
const MEMORY_EXT: &str = "unipi.memory";
const INVENTORY: &str = "memory.inventory";

/// What the dream prompt points at.
#[derive(Clone, Debug, Default)]
pub struct PromptInputs<'a> {
    /// Staging dir (host path).
    pub staging: &'a str,
    /// Memory root (host path).
    pub memory_root: Option<&'a str>,
    /// Memory projects covered.
    pub projects: &'a [String],
    /// `memory/before.json` was written.
    pub has_inventory: bool,
    /// Prune window (days, `0` = never prune).
    pub stale_days: f64,
}

/// The dream prompt (unipi `dreamPrompt`, v3 Devin consolidation); paths are host paths.
#[must_use]
pub fn dream_prompt(i: &PromptInputs<'_>) -> String {
    let s = i.staging;
    let projects = if i.projects.is_empty() {
        "(none: no recent sessions; still consolidate and prune what memory/before.json lists)"
            .to_owned()
    } else {
        i.projects.join(", ")
    };
    let inventory = if i.has_inventory {
        format!(
            "- {s}/memory/before.json: every memory of the covered projects: project, id, title, type, updated, snippet, file (read it for the full text), useCount, lastUsed, sessions (that used it), sources (that wrote it), stale (true = no session searched, read or was shown it for {} days).",
            store::fmt_num(i.stale_days)
        )
    } else {
        "- The memory inventory is unavailable this run: use memory_list with `project` for each covered project, and treat no memory as stale.".to_owned()
    };
    let prune = if i.stale_days > 0.0 && i.has_inventory {
        "4. Prune stale memories: a memory marked \"stale\": true that is transient, obsolete or contradicted by the sessions -> memory_delete. Never delete a `preference` or an explicit user instruction; when unsure, keep it.".to_owned()
    } else {
        "4. Pruning is off this run: delete only memories you merged or that the sessions plainly contradict.".to_owned()
    };
    [
        "You are running a DREAM pass: Oino's offline review of recent coding-agent sessions and the memories they left behind. Nobody is watching; work only with files and the memory tools.".to_owned(),
        String::new(),
        "Memories are short lessons learned from working with this user: preferences, corrections, project decisions, environment gotchas, each linked to the session it came from. They are NOT session summaries. Keep the memory index small, true and useful for the next session.".to_owned(),
        String::new(),
        "Inputs (read-only):".to_owned(),
        format!("- {s}/digests/INDEX.json: `projects` (memory project, cwd, sessions) and `sessions` (every recent session with its project and first request; sessions with events add errors, corrections, preferences, memoryStores and a `digest` file under {s}/digests/). A digest lists failed tool calls (call, errorHead = first line(s) of the error, errorTail = end of the error, recovery = the next successful call of the same tool), user corrections and stated user preferences."),
        inventory,
        format!("- Projects covered: {projects}."),
        String::new(),
        "Do these four things, in order:".to_owned(),
        "1. Consolidate: memories in one project that overlap or say the same thing become ONE memory. memory_store the merged note with `merge_from` = the titles of the notes it replaces (they are deleted and their usage history carries over). Keep the clearest wording and every source reference.".to_owned(),
        "2. Remove transient details: facts about one moment (a port 'this afternoon', a temp path, a branch or task that is finished, 'today', 'currently') -> rewrite the note into the durable lesson behind it, or delete it when nothing durable remains.".to_owned(),
        "3. Find missed lessons: a false path in the digests (an error, then a recovery with a different call), a user correction, or a stated preference that no memory captures yet. Group the same false path across sessions and pick the cheapest fix:".to_owned(),
        "   - seen in 1 session -> a memory lesson, or nothing if it was a one-off".to_owned(),
        "   - seen in 2+ sessions and mechanical (fixed command shape, schema, banned pattern) -> propose an automated check or a one-line lesson keyed by the error text".to_owned(),
        "   - seen in 2+ sessions and a multi-step procedure -> propose a skill (write it with the craft-skill skill, loaded for this run)".to_owned(),
        "   - a stated preference -> a `preference` memory in the user's own words, with the reason they gave".to_owned(),
        prune,
        String::new(),
        i.memory_root.map_or_else(
            || "Memory: use the memory tools (memory_store / memory_delete / memory_list) and always pass `project` (the memory's project from the inputs): your working directory is a staging folder, not a project.".to_owned(),
            |m| format!("Memory: use the memory tools (memory_store / memory_delete / memory_list) so the search index stays in sync, and always pass `project` (the memory's project from the inputs): your working directory is a staging folder, not a project. The memory files live under {m}; read them, never edit them directly."),
        ),
        String::new(),
        "Rules:".to_owned(),
        "- Every memory you write keeps or adds its source line(s): `[source: <session id>]` (session ids are the session names in the digest index). When merging, carry every source of the merged notes. No session id, no new lesson.".to_owned(),
        "- A lesson is one line: `When <error text or situation>, <shortest path>. [source: <session id>]`".to_owned(),
        "- Keep explicit user preferences and their reasons; never weaken or drop them.".to_owned(),
        "- Never copy a secret, token, or password into any file.".to_owned(),
        format!("- Write skill and check proposals only under {s}/proposals/, never anywhere else. Skills are crafted with the craft-skill skill (scaffold + check scripts). Name the project each proposal is for in {s}/proposals/projects.json: {{\"<proposal file or folder name>\": \"<project cwd from the digest index>\"}}."),
        format!("- Record every memory change in {s}/memory-changes.json: a JSON array of {{\"action\": \"merged|added|rewritten|pruned\", \"project\", \"title\", \"from\": [titles merged in], \"reason\", \"sources\": [session ids]}}."),
        String::new(),
        format!("Done when {s}/DREAM_REPORT.md exists with three sections: ## Memory edits (applied; one bullet per change, starting with merged / added / rewritten / pruned), ## Proposals (waiting for approval, each with its sources), ## Skipped (groups you judged one-off, with why)."),
    ]
    .join("\n")
}

/// unipi memory project-name rule.
#[must_use]
pub fn memory_project(cwd: &str) -> String {
    let base = cwd
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let mut out = String::new();
    let mut gap = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

/// Result of an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Success.
    pub ok: bool,
    /// Message.
    pub message: String,
}

fn ok(m: impl Into<String>) -> Outcome {
    Outcome {
        ok: true,
        message: m.into(),
    }
}
fn fail(m: impl Into<String>) -> Outcome {
    Outcome {
        ok: false,
        message: m.into(),
    }
}

/// The extension.
#[derive(Default)]
pub struct Dream {
    settings: Store,
    agents: RefCell<HashMap<String, Rc<Agent>>>,
    views: SessionMap<View>,
    /// Last overdue check by an interactive session (unix ms).
    last_auto: Cell<i64>,
    /// Resume of interrupted runs already attempted in this process.
    resumed: Cell<bool>,
    /// The last schedule sync error.
    sched_error: RefCell<Option<String>>,
    /// Runs whose agent is closing (still "in flight" for `run --scheduled`).
    closing: RefCell<Vec<String>>,
}

fn spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

fn host_of(p: &Path) -> String {
    unipi_kit::paths::host_path(p)
        .map_or_else(|| p.display().to_string(), |h| h.display().to_string())
}

fn agent_dir() -> Option<String> {
    oino_extension::fs::preopens()
        .into_iter()
        .find(|p| p.guest_path.starts_with("/agent/"))
        .and_then(|p| p.host_path.rsplit_once('/').map(|(a, _)| a.to_owned()))
}

fn append_log(staging: &Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(staging.join("trajectory.log"))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// The agent dir's memory root (host path).
fn memory_root() -> Option<String> {
    agent_dir().map(|a| format!("{a}/state/unipi.memory/global/memory"))
}

/// `memory.inventory` from `unipi.memory` (`None` when the extension is off or slow).
async fn inventory(projects: &[String], stale_days: f64) -> Option<Value> {
    services::call(
        Some(MEMORY_EXT),
        INVENTORY,
        &json!({"projects": projects, "staleDays": stale_days}),
        Some(Duration::from_secs(60)),
    )
    .await
    .ok()
    .filter(|v| v.get("memories").is_some_and(Value::is_array))
}

fn write_json(p: &Path, v: &Value) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_vec_pretty(v).unwrap_or_default());
}

/// Writes `memory/after.json` and `changes.json` for a finished run.
async fn finalize(staging: &Path, projects: &[String], stale_days: f64) {
    let before: Option<Value> = std::fs::read(staging.join("memory/before.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(before) = before else {
        return;
    };
    let Some(after) = inventory(projects, stale_days).await else {
        append_log(
            staging,
            "memory inventory unavailable after the run; no change list",
        );
        return;
    };
    write_json(&staging.join("memory/after.json"), &after);
    let mut changes = store::diff_inventory(&before, &after);
    changes["actions"] = json!(store::read_actions(staging));
    write_json(&staging.join("changes.json"), &changes);
}

impl Dream {
    fn cfg(&self) -> DreamSettings {
        settings::normalize(&self.settings.get(NAMESPACE).0)
    }

    /// Where new runs live (one dream across projects).
    fn root() -> PathBuf {
        unipi_kit::paths::global("runs/all")
    }

    /// Every run dir (`runs/all` + legacy `runs/<wsid>`).
    fn runs_root() -> PathBuf {
        unipi_kit::paths::global("runs")
    }

    fn state_path() -> PathBuf {
        unipi_kit::paths::global("state.json")
    }

    /// The global state, with the current workspace's legacy state folded in once.
    fn read_state() -> DreamState {
        let p = Self::state_path();
        let mut st = store::read_state(&p);
        let legacy_p = unipi_kit::paths::workspace("state.json");
        if legacy_p.exists() {
            let legacy = store::read_state(&legacy_p);
            st.absorb_legacy(&legacy);
            store::write_state(&p, &st);
            let _ = std::fs::rename(&legacy_p, legacy_p.with_extension("json.migrated"));
        }
        st
    }

    fn sessions_root() -> PathBuf {
        PathBuf::from("/agent/sessions")
    }

    fn skip() -> digest::Skip {
        digest::Skip::for_agent_dir(agent_dir().as_deref())
    }

    /// Run ids alive in this process or (by heartbeat) in another one.
    fn alive(&self) -> impl Fn(&str) -> bool + '_ {
        let beating = store::heartbeat_ids(&Self::runs_root(), timers::now_ms());
        move |id| self.agents.borrow().contains_key(id) || beating.iter().any(|b| b == id)
    }

    fn runs(&self) -> Vec<Run> {
        let st = Self::read_state();
        store::list_all_runs(&Self::runs_root(), &st, timers::now_ms(), &self.alive())
    }

    fn changed(&self) {
        unipi_kit::bus::emit_raw(topics::WORK_TRAY_CHANGED, &json!({"tab": "dream"}));
        if self.agents.borrow().is_empty() {
            timers::cancel(POLL_TIMER);
        }
    }

    fn agent_spec(cfg: &DreamSettings, staging: &str, seed: Option<&str>) -> Value {
        let mut spec = json!({
            "label": "dream",
            "tools": {"builtin": ["read", "bash", "edit", "write", "grep", "find", "ls"], "extensionTools": ["memory_store", "memory_delete", "memory_search", "memory_list"], "deny": []},
            "extensions": {"only": ["unipi.memory"]},
            "seed": seed.map_or_else(|| json!("none"), |f| json!({"sessionFile": f})),
            "persistence": {"file": {"sessionDir": format!("{staging}/sessions")}},
            "cwd": staging,
            "childRole": "dream",
            "ui": "none",
            "maxDepth": 1,
            "thinking": cfg.thinking,
        });
        if !cfg.model.is_empty() {
            spec["model"] = json!(cfg.model);
        }
        spec
    }

    /// Spawns the agent for `staging` and watches it in the background: heartbeat, stop
    /// requests from any process, the runtime cap, then the memory change list.
    async fn launch(
        &'static self,
        cfg: &DreamSettings,
        staging: &Path,
        seed: Option<String>,
        prompt: &str,
    ) -> std::result::Result<String, String> {
        let id = staging
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let host = host_of(staging);
        let agent = Agent::spawn(&Self::agent_spec(cfg, &host, seed.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
        append_log(staging, &format!("dream agent {} started", agent.id()));
        store::patch_meta(staging, |m| m.heartbeat_at = Some(timers::now_ms()));
        agent.prompt(prompt).await.map_err(|e| e.to_string())?;
        let agent = Rc::new(agent);
        self.agents
            .borrow_mut()
            .insert(id.clone(), Rc::clone(&agent));
        timers::set(POLL_TIMER, Duration::from_secs(2), true);
        let max = Duration::from_secs_f64(cfg.max_runtime_min * 60.0);
        let stale_days = cfg.stale_days;
        let staging = staging.to_path_buf();
        let run_id = id.clone();
        spawn(async move {
            let (code, error) = {
                let timeout = timers::sleep(max).fuse();
                let idle = agent.wait_idle().fuse();
                let beat_dir = staging.clone();
                let beat = async move {
                    loop {
                        #[allow(clippy::cast_sign_loss, reason = "positive constant")]
                        timers::sleep(Duration::from_millis(store::HEARTBEAT_MS as u64)).await;
                        if store::read_meta(&beat_dir).is_none_or(|m| m.stopped) {
                            break;
                        }
                        store::patch_meta(&beat_dir, |m| m.heartbeat_at = Some(timers::now_ms()));
                    }
                }
                .fuse();
                futures::pin_mut!(timeout, idle, beat);
                futures::select! {
                    r = idle => match r {
                        Ok(()) => (0, None),
                        Err(e) => (1, Some(e.to_string())),
                    },
                    () = timeout => (124, None),
                    () = beat => (130, None),
                }
            };
            if code == 124 || code == 130 {
                agent.abort();
            }
            if let Some(t) = agent.last_assistant_text() {
                append_log(&staging, &t);
            }
            if let Some(e) = &error {
                append_log(&staging, &format!("Error: {e}"));
            }
            let meta = store::read_meta(&staging).unwrap_or_default();
            let stopped = meta.stopped;
            store::patch_meta(&staging, |m| {
                m.ended_at.get_or_insert(timers::now_ms());
                m.heartbeat_at = None;
                if !stopped {
                    m.exit_code = Some(code);
                    m.error.clone_from(&error);
                }
            });
            finalize(&staging, &meta.projects, stale_days).await;
            self.closing.borrow_mut().push(id.clone());
            let held = self.agents.borrow_mut().remove(&id);
            drop(held);
            if let Ok(a) = Rc::try_unwrap(agent) {
                a.close(5_000).await;
            }
            self.closing.borrow_mut().retain(|c| c != &id);
            let st_path = Self::state_path();
            let mut st = store::read_state(&st_path);
            if st.lock.as_ref().and_then(|l| l.run.as_deref()) == Some(id.as_str()) {
                st.lock = None;
                store::write_state(&st_path, &st);
            }
            self.changed();
        });
        Ok(run_id)
    }

    /// unipi `startDream`, v3: staging dir, cross-project digests, memory inventory,
    /// prompt, agent. Returns `(run id, sessions looked at)`.
    async fn start(&'static self, trigger: &str) -> std::result::Result<(String, u32), String> {
        let cfg = self.cfg();
        let started = timers::now_ms();
        let st = Self::read_state();
        let since = if st.last_run_at > 0 {
            st.last_run_at
        } else {
            started - FIRST_WINDOW_MS
        };
        let staging = Self::root().join(format!("staging-{started}"));
        std::fs::create_dir_all(staging.join("proposals")).map_err(|e| e.to_string())?;
        let recent = digest::digest_recent(
            &Self::sessions_root(),
            since,
            &staging.join("digests"),
            &Self::skip(),
            MAX_SESSIONS,
            &memory_project,
        );
        let projects: Vec<String> = recent
            .projects
            .iter()
            .take(MAX_PROJECTS)
            .map(|p| p.project.clone())
            .collect();
        let inv = inventory(&projects, cfg.stale_days).await;
        if let Some(v) = &inv {
            write_json(&staging.join("memory/before.json"), v);
        }
        let host = host_of(&staging);
        let root = memory_root();
        let prompt = dream_prompt(&PromptInputs {
            staging: &host,
            memory_root: root.as_deref(),
            projects: &projects,
            has_inventory: inv.is_some(),
            stale_days: cfg.stale_days,
        });
        let _ = std::fs::write(staging.join("PROMPT.md"), &prompt);
        store::write_meta(
            &staging,
            &RunMeta {
                started_at: started,
                pid: -1,
                manual: trigger == "manual",
                sessions: recent.sessions,
                events: recent.events,
                trigger: Some(trigger.to_owned()),
                projects: projects.clone(),
                ..RunMeta::default()
            },
        );
        let id = match self.launch(&cfg, &staging, None, &prompt).await {
            Ok(id) => id,
            Err(e) => {
                store::patch_meta(&staging, |m| {
                    m.ended_at = Some(timers::now_ms());
                    m.exit_code = Some(-1);
                    m.error = Some(e.clone());
                });
                append_log(&staging, &format!("Error: {e}"));
                return Err(e);
            }
        };
        let p = Self::state_path();
        let mut st = store::read_state(&p);
        st.last_run_at = started;
        st.lock = Some(Lock {
            pid: 0,
            at: started,
            run: Some(id.clone()),
        });
        st.shown_report = None;
        st.sessions_seen = recent.sessions;
        store::write_state(&p, &st);
        self.changed();
        Ok((id, recent.sessions))
    }

    /// Resumes runs a previous process left unfinished (once each).
    async fn resume_interrupted(&'static self) {
        let cfg = self.cfg();
        for run in self.runs().into_iter().filter(|r| r.interrupted) {
            let meta = store::read_meta(&run.staging).unwrap_or_default();
            let seed = store::resume_seed(&run.staging).map(|p| host_of(&p));
            if meta.resumes >= 1 || seed.is_none() {
                store::patch_meta(&run.staging, |m| {
                    m.ended_at = Some(timers::now_ms());
                    m.heartbeat_at = None;
                    m.exit_code = Some(1);
                    m.error = Some("interrupted (Oino exited during the dream)".into());
                });
                continue;
            }
            store::patch_meta(&run.staging, |m| m.resumes += 1);
            let host = host_of(&run.staging);
            let root = memory_root();
            let prompt = format!(
                "The dream pass was interrupted. Continue from where you stopped.\n\n{}",
                dream_prompt(&PromptInputs {
                    staging: &host,
                    memory_root: root.as_deref(),
                    projects: &meta.projects,
                    has_inventory: run.staging.join("memory/before.json").exists(),
                    stale_days: cfg.stale_days,
                })
            );
            if let Err(e) = self.launch(&cfg, &run.staging, seed, &prompt).await {
                store::patch_meta(&run.staging, |m| {
                    m.ended_at = Some(timers::now_ms());
                    m.exit_code = Some(-1);
                    m.error = Some(e);
                });
            }
        }
    }

    fn stop(&self, id: Option<&str>) -> Outcome {
        let run = self
            .runs()
            .into_iter()
            .find(|r| r.status == Status::Running && id.is_none_or(|i| r.id == i));
        let Some(run) = run else {
            return fail("No dream is running.");
        };
        // The owner (this or another process) sees `stopped` on its next heartbeat.
        store::patch_meta(&run.staging, |m| {
            m.stopped = true;
            m.ended_at = Some(timers::now_ms());
            m.heartbeat_at = None;
        });
        if let Some(a) = self.agents.borrow().get(&run.id) {
            a.abort();
        }
        let p = Self::state_path();
        let mut st = store::read_state(&p);
        if st.lock.as_ref().and_then(|l| l.run.as_deref()) == Some(run.id.as_str()) {
            st.lock = None;
            store::write_state(&p, &st);
        }
        self.changed();
        ok("Dream stopped.")
    }

    async fn run_now(&'static self) -> Outcome {
        if self.runs().iter().any(|r| r.status == Status::Running) {
            return fail("A dream is already running.");
        }
        match self.start("manual").await {
            Ok((_, sessions)) => ok(format!(
                "Dream running in the background ({sessions} recent sessions to review)."
            )),
            Err(e) => fail(format!("Couldn't start the dream agent: {e}")),
        }
    }

    fn record_check(trigger: &str, ran: bool, reason: &str) {
        let p = Self::state_path();
        let mut st = store::read_state(&p);
        st.push_check(LastCheck {
            at: timers::now_ms(),
            trigger: trigger.to_owned(),
            ran,
            reason: reason.to_owned(),
        });
        store::write_state(&p, &st);
    }

    /// A gated dream (`scheduled` from the host cron, `catch-up` from a session).
    /// `wait` keeps the call alive until the agent ends (the headless cron process exits
    /// when the command returns).
    async fn run_gated(&'static self, trigger: &str, wait: bool) -> Outcome {
        let cfg = self.cfg();
        let st = Self::read_state();
        let d = self.due(&st, &cfg);
        if !d.due {
            Self::record_check(trigger, false, &d.reason);
            return ok(format!("Dream skipped: {}.", d.reason));
        }
        if self.runs().iter().any(|r| r.status == Status::Running) {
            Self::record_check(trigger, false, "a dream is already running");
            return ok("Dream skipped: a dream is already running.");
        }
        let (id, sessions) = match self.start(trigger).await {
            Ok(v) => v,
            Err(e) => {
                Self::record_check(trigger, false, &format!("couldn't start: {e}"));
                return fail(format!("Couldn't start the dream agent: {e}"));
            }
        };
        Self::record_check(trigger, true, &d.reason);
        if !wait {
            return ok(format!("Dream running ({sessions} recent sessions)."));
        }
        while self.agents.borrow().contains_key(&id) || self.closing.borrow().contains(&id) {
            timers::sleep(Duration::from_secs(2)).await;
        }
        let run = self.runs().into_iter().find(|r| r.id == id);
        ok(match run {
            Some(r) if r.status == Status::Finished => format!(
                "Dream finished: {}.",
                store::summarize_report(&store::read_report(&r.staging)).join(", ")
            ),
            Some(r) => format!(
                "Dream ended {}{}.",
                r.status.as_str(),
                r.error.map(|e| format!(": {e}")).unwrap_or_default()
            ),
            None => "Dream ended.".to_owned(),
        })
    }

    /// Writes / removes the host cron job to match the settings; `Err` = why it couldn't.
    async fn sync_schedule(&self) -> std::result::Result<bool, String> {
        let cfg = self.cfg();
        let path = schedule::job_path();
        let existing: Option<Value> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let created = existing
            .as_ref()
            .and_then(|e| e.get("created"))
            .and_then(Value::as_u64);
        let cwd = agent_dir().unwrap_or_default();
        let now = u64::try_from(timers::now_ms()).unwrap_or(0);
        let result = match schedule::job_value(&cfg, &cwd, now, created) {
            None => {
                if existing.is_some() {
                    std::fs::remove_file(&path)
                        .map(|()| false)
                        .map_err(|e| format!("couldn't remove the dream job: {e}"))
                } else {
                    Ok(false)
                }
            }
            Some(job)
                if existing
                    .as_ref()
                    .is_some_and(|e| schedule::same_job(e, &job)) =>
            {
                Ok(true)
            }
            Some(job) => {
                if Path::new(schedule::JOBS_DIR).exists() {
                    let _guard = oino_extension::fs::Lock::acquire(schedule::LOCK, true)
                        .await
                        .ok();
                    oino_extension::fs::atomic_write(
                        &path,
                        &serde_json::to_vec_pretty(&job).unwrap_or_default(),
                    )
                    .map(|()| true)
                    .map_err(|e| format!("couldn't write the dream job: {e}"))
                } else {
                    Err("the host cron folder (~/.oino/cron/jobs) is missing: start the Oino host once".to_owned())
                }
            }
        };
        *self.sched_error.borrow_mut() = result.as_ref().err().cloned();
        result
    }

    /// The host scheduler's view of the dream job.
    async fn scheduler_json(&self, cfg: &DreamSettings) -> Value {
        let registered = self.sync_schedule().await;
        let host = host_service::available("cron");
        let mut v = json!({
            "jobId": schedule::JOB_ID,
            "registered": registered.as_ref().is_ok_and(|r| *r),
            "hostRunning": host,
            "cron": schedule::cron_expr(cfg),
            "nextRun": Value::Null,
        });
        if let Err(e) = &registered {
            v["error"] = json!(e);
        }
        if host
            && registered.is_ok_and(|r| r)
            && let Ok(show) =
                host_service::call("cron", "show", &json!({"id": schedule::JOB_ID})).await
        {
            v["nextRun"] = show
                .pointer("/job/next_fire")
                .cloned()
                .unwrap_or(Value::Null);
            v["jobRunning"] = show.pointer("/job/running").cloned().unwrap_or(Value::Null);
            if let Some(last) = show.pointer("/runs/0") {
                v["lastFire"] = json!({
                    "runId": last.get("id"),
                    "status": last.get("status"),
                    "firedAt": last.get("fired_at"),
                    "reason": last.get("reason"),
                });
            }
        }
        v
    }

    fn find(&self, run_id: &str, proposal_id: &str) -> Option<(Run, store::Proposal)> {
        let run = self.runs().into_iter().find(|r| r.id == run_id)?;
        let p = run.proposals.iter().find(|p| p.id == proposal_id)?.clone();
        Some((run, p))
    }

    fn decide(&self, run_id: &str, proposal_id: &str, approve: bool, cwd: &str) -> Outcome {
        let Some((_, p)) = self.find(run_id, proposal_id) else {
            return fail("That proposal no longer exists.");
        };
        if p.decision != "pending" {
            return fail(format!("{} is already {}.", p.name, p.decision));
        }
        let record = |d: &str| {
            let path = Self::state_path();
            let mut st = store::read_state(&path);
            st.decisions.insert(proposal_id.to_owned(), d.to_owned());
            store::write_state(&path, &st);
        };
        if !approve {
            record("rejected");
            self.changed();
            return ok(format!("Rejected {}.", p.name));
        }
        // A cross-project dream's skill lands in the project it was proposed for; the
        // guest can only write the calling session's project, so the caller must be there.
        if p.kind == "skill"
            && let Some(target) = p.cwd.as_deref()
            && target.trim_end_matches(['/', '\\']) != cwd.trim_end_matches(['/', '\\'])
        {
            return fail(format!(
                "{} is for {target}: approve it from a session in that project (the skill lands in its {}).",
                p.name,
                self.cfg().skills_target
            ));
        }
        let result: std::result::Result<String, String> = if p.kind == "skill" {
            let target_rel = format!(
                "{}/{}",
                self.cfg().skills_target.trim_end_matches('/'),
                p.name
            );
            let host_target = pathx_resolve(cwd, &target_rel);
            let guest = Path::new("/project").join(target_rel.trim_start_matches('/'));
            #[allow(clippy::single_match_else, reason = "copy then check")]
            match store::copy_dir(&p.path, &guest) {
                Err(e) => Err(format!("{e}")),
                Ok(()) => {
                    let c = store::craft_check(&guest, &host_target);
                    let detail = if c.output.is_empty() {
                        format!("copied to {host_target} (check script unavailable)")
                    } else {
                        c.output
                    };
                    if c.ok { Ok(detail) } else { Err(detail) }
                }
            }
        } else {
            store::check_head(&p)
                .map(|h| format!("approved. Wire it as an Oino hook/extension yourself:\n{h}"))
                .ok_or_else(|| format!("{}: unreadable", p.path.display()))
        };
        self.changed();
        match result {
            Ok(detail) => {
                record("approved");
                ok(format!("Approved {}. {detail}", p.name).trim().to_owned())
            }
            Err(detail) => fail(format!("Approval failed (not recorded): {detail}")),
        }
    }

    fn dismiss(&self, run_id: &str) -> Outcome {
        let Some(run) = self.runs().into_iter().find(|r| r.id == run_id) else {
            return fail("That dream is gone.");
        };
        if run.status == Status::Running {
            return fail("That dream is still running — stop it first.");
        }
        let p = Self::state_path();
        let mut st = store::read_state(&p);
        if !st.dismissed.contains(&run.id) {
            st.dismissed.push(run.id);
        }
        store::write_state(&p, &st);
        self.changed();
        ok("Dismissed.")
    }

    async fn status_json(&self) -> Value {
        let cfg = self.cfg();
        let st = Self::read_state();
        let runs = self.runs();
        let mut v = json!({
            "v": 2,
            "enabled": cfg.enabled,
            "schedule": {
                "mode": cfg.schedule.mode.as_str(),
                "time": cfg.schedule.time_str(),
                "intervalHours": cfg.schedule.interval_hours,
            },
            "minSessions": cfg.min_sessions,
            "minGapHours": cfg.min_gap_hours,
            "staleDays": cfg.stale_days,
            "model": cfg.model,
            "thinking": cfg.thinking,
            "maxRuntimeMin": cfg.max_runtime_min,
            "lastRunAt": st.last_run_at,
            "lastCheck": st.last_check,
            "checks": st.checks,
            "running": runs.iter().any(|r| r.status == Status::Running),
            "runs": runs.iter().take(20).map(store::run_info).collect::<Vec<_>>(),
            "scheduler": self.scheduler_json(&cfg).await,
        });
        if cfg.enabled {
            let d = self.due(&st, &cfg);
            v["due"] = json!({"due": d.due, "reason": d.reason, "newSessions": d.new_sessions});
        }
        v
    }

    fn due(&self, st: &DreamState, cfg: &DreamSettings) -> store::Due {
        let n = digest::count_recent(&Self::sessions_root(), st.last_run_at, &Self::skip());
        store::is_due(st, n, cfg, timers::now_ms(), &self.alive())
    }

    fn schedule_text(cfg: &DreamSettings) -> String {
        if !cfg.enabled {
            return "off (manual only)".into();
        }
        match cfg.schedule.mode {
            Mode::Daily => format!("daily at {}", cfg.schedule.time_str()),
            Mode::Interval => format!(
                "every {}h (minute {:02})",
                store::fmt_num(cfg.schedule.interval_hours.round()),
                cfg.schedule.time.1
            ),
            Mode::Manual => "manual only".into(),
        }
    }

    fn status_text(&self) -> String {
        let cfg = self.cfg();
        let st = Self::read_state();
        let runs = self.runs();
        let latest = runs.iter().find(|r| r.has_report);
        let sched_err = self.sched_error.borrow().clone();
        let mut lines = vec![
            if cfg.enabled {
                let d = self.due(&st, &cfg);
                format!(
                    "dream: {} · {}",
                    Self::schedule_text(&cfg),
                    if d.due {
                        "gates pass".to_owned()
                    } else {
                        format!("next one would skip: {}", d.reason)
                    }
                )
            } else {
                "dream: off — turn on in /settings → Dream (manual /dream run still works)"
                    .to_owned()
            },
            format!(
                "last run {}{}",
                if st.last_run_at > 0 {
                    unipi_kit::fmt::iso_from_ms(st.last_run_at)
                } else {
                    "never".into()
                },
                if runs.iter().any(|r| r.status == Status::Running) {
                    " · a dream is running now"
                } else {
                    ""
                }
            ),
            format!("state: {}", host_of(&Self::state_path())),
            latest.map_or_else(
                || "no report yet".to_owned(),
                |l| {
                    format!(
                        "latest report: {}",
                        host_of(&l.staging.join("DREAM_REPORT.md"))
                    )
                },
            ),
        ];
        if let Some(c) = &st.last_check {
            lines.push(format!(
                "last {} check {}: {}",
                c.trigger,
                unipi_kit::fmt::iso_from_ms(c.at),
                if c.ran { "ran" } else { c.reason.as_str() }
            ));
        }
        if let Some(e) = sched_err {
            lines.push(format!("schedule: {e}"));
        }
        if let Some(l) = latest {
            for (i, p) in l.proposals.iter().enumerate() {
                lines.push(format!(
                    "  {}. [{}] {} — {}",
                    i + 1,
                    p.kind,
                    p.name,
                    p.decision
                ));
            }
            if l.proposals.iter().any(|p| p.decision == "pending") {
                lines.push("approve with /dream approve <n> (or the work tray's Dream tab)".into());
            }
        }
        lines.join("\n")
    }

    async fn on_session_start(&'static self, cx: &Ctx) {
        self.settings.refresh();
        // Only interactive top-level sessions do upkeep: a headless `oino -p` (the scheduled
        // dream itself, other cron jobs) exits right after its prompt.
        if cx.is_child() || !cx.has_ui() {
            return;
        }
        let _ = self.sync_schedule().await;
        if !self.resumed.replace(true) {
            self.resume_interrupted().await;
        }
        let p = Self::state_path();
        let st = Self::read_state();
        let latest = self.runs().into_iter().find(|r| r.has_report);
        if let Some(run) = latest
            && st.shown_report.as_deref() != Some(run.id.as_str())
            && !st.dismissed.contains(&run.id)
        {
            let _ = session::append_entry(
                REPORT_CARD,
                Some(
                    &json!({"report": host_of(&run.staging.join("DREAM_REPORT.md")), "pending": run.pending}),
                ),
            );
            let mut st = st;
            st.shown_report = Some(run.id);
            store::write_state(&p, &st);
            return;
        }
        // Catch-up: a scheduled dream was missed (no host scheduler was running then). A
        // recent host-scheduled check that skipped on its gates counts as on time.
        let now = timers::now_ms();
        if now - self.last_auto.get() < AUTO_CHECK_MS {
            return;
        }
        self.last_auto.set(now);
        let cfg = self.cfg();
        if schedule::overdue(&cfg, st.last_scheduled_activity(), now) {
            let _ = self.run_gated("catch-up", false).await;
        }
    }

    // ── work tray ────────────────────────────────────────────────────────────

    fn tab(&self, req: &TabRequest, cx: &Ctx) -> Value {
        let runs = self.runs();
        let cfg = self.cfg();
        if let Some(id) = req
            .initial_id
            .as_ref()
            .filter(|i| runs.iter().any(|r| &r.id == *i))
        {
            self.views.with(cx.session_id(), |v| {
                *v = View {
                    open: Some(id.clone()),
                    ..View::default()
                };
            });
        }
        let counts = Counts {
            total: u32::try_from(runs.len()).unwrap_or(u32::MAX),
            running: u32::try_from(runs.iter().filter(|r| r.status == Status::Running).count())
                .unwrap_or(u32::MAX),
        };
        let body = req.open.then(|| {
            let view = self.views.with(cx.session_id(), |v| {
                let snapshot = v.clone();
                v.flash = None;
                snapshot
            });
            let now = timers::now_ms();
            match view
                .open
                .as_ref()
                .and_then(|id| runs.iter().find(|r| &r.id == id))
            {
                Some(run) => {
                    let (steps, log) = store::run_trajectory(&run.staging, 200);
                    tray::detail_body(
                        run,
                        &store::read_report(&run.staging),
                        &steps,
                        &log,
                        &view,
                        now,
                    )
                }
                None => tray::list_body(&runs, cfg.enabled, &view, now),
            }
        });
        json!(TrayTab {
            id: "dream".into(),
            label: "Dream".into(),
            short_label: None,
            order: 2,
            counts,
            visible: !runs.is_empty() || cfg.enabled,
            body,
            owner: None,
            surfaces: None,
        })
    }

    #[allow(clippy::too_many_lines, reason = "one arm per tray key")]
    async fn tray_action(&'static self, a: &Action, cx: &Ctx) -> ActionReply {
        let sid = cx.session_id().to_owned();
        let cwd = cx.cwd().display().to_string();
        let view = self.views.with(&sid, |v| v.clone());
        let current = view
            .open
            .as_ref()
            .and_then(|id| self.runs().into_iter().find(|r| &r.id == id));
        let flash = |m: String| self.views.with(&sid, |v| v.flash = Some(m));
        let digit = a
            .action_id
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=9).contains(n));
        match a.action_id.as_str() {
            "open" => {
                let id = a
                    .value
                    .as_ref()
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let pick = id
                    .as_ref()
                    .and_then(|i| self.runs().into_iter().find(|r| &r.id == i))
                    .and_then(|r| r.proposals.iter().position(|p| p.decision == "pending"))
                    .unwrap_or(0);
                self.views.with(&sid, |v| {
                    *v = View {
                        open: id,
                        pick,
                        ..View::default()
                    };
                });
            }
            "back" => self.views.with(&sid, |v| *v = View::default()),
            "report" => {
                if view.open.is_none() {
                    if let Some(r) = self.runs().into_iter().find(|r| r.has_report) {
                        self.views.with(&sid, |v| {
                            *v = View {
                                open: Some(r.id),
                                report: true,
                                ..View::default()
                            };
                        });
                    }
                } else {
                    self.views.with(&sid, |v| v.report = !v.report);
                }
            }
            "run" => flash(self.run_now().await.message),
            "stop" => flash(self.stop(view.open.as_deref()).message),
            "dismiss" => {
                if let Some(run) = &current {
                    let r = self.dismiss(&run.id);
                    if r.ok {
                        self.views.with(&sid, |v| *v = View::default());
                    }
                    flash(r.message);
                }
            }
            "pick" => {
                if let (Some(run), Some(id)) = (&current, a.value.as_ref().and_then(Value::as_str))
                {
                    let i = run.proposals.iter().position(|p| p.id == id).unwrap_or(0);
                    self.views.with(&sid, |v| v.pick = i);
                }
            }
            verb @ ("approve" | "reject") => {
                let Some(run) = &current else {
                    return ActionReply {
                        close: false,
                        refresh: true,
                    };
                };
                let Some(p) = run.proposals.get(view.pick) else {
                    return ActionReply {
                        close: false,
                        refresh: true,
                    };
                };
                let key = if verb == "approve" { "a" } else { "x" };
                if view.armed.as_ref() == Some(&(key.to_owned(), p.id.clone())) {
                    let r = self.decide(&run.id, &p.id, verb == "approve", &cwd);
                    self.views.with(&sid, |v| v.armed = None);
                    flash(r.message);
                } else {
                    self.views
                        .with(&sid, |v| v.armed = Some((key.to_owned(), p.id.clone())));
                    flash(format!("press {key} again to {verb} {}", p.name));
                }
            }
            _ if digit.is_some() => {
                if let (Some(run), Some(n)) = (&current, digit)
                    && n <= run.proposals.len()
                {
                    self.views.with(&sid, |v| v.pick = n - 1);
                }
            }
            _ => {}
        }
        ActionReply {
            close: false,
            refresh: true,
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per subcommand")]
    async fn command(&'static self, args: &str, cx: &CommandCtx) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let sub = parts.first().copied().unwrap_or("status");
        let cwd = cx.cx.cwd().display().to_string();
        let say = |t: &str, level: &str| {
            if cx.has_ui() {
                let _ = ui::notify(t, level);
            }
        };
        match sub {
            "status" => say(&self.status_text(), "info"),
            "tray" | "open" => {
                let opened = cx.has_ui()
                    && services::call(
                        Some("unipi.core"),
                        point::CORE_OPEN_TRAY,
                        &json!({"tab": "dream"}),
                        None,
                    )
                    .await
                    .ok()
                    .and_then(|v| v.get("opened").and_then(Value::as_bool))
                    .unwrap_or(false);
                if !opened {
                    say(&self.status_text(), "info");
                }
            }
            "report" => match self.runs().into_iter().find(|r| r.has_report) {
                None => say("no dream report yet", "info"),
                Some(r) => match std::fs::read_to_string(r.staging.join("DREAM_REPORT.md")) {
                    Ok(t) => say(&t, "info"),
                    Err(_) => say("report unreadable", "error"),
                },
            },
            "approve" | "reject" => {
                let Some(latest) = self.runs().into_iter().find(|r| r.has_report) else {
                    return say("no proposals yet", "info");
                };
                let n = parts
                    .get(1)
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| *n >= 1);
                let Some(p) = n.and_then(|n| latest.proposals.get(n - 1)) else {
                    return say(
                        &format!("usage: /dream {sub} <1-{}>", latest.proposals.len()),
                        "warning",
                    );
                };
                let r = self.decide(&latest.id, &p.id, sub == "approve", &cwd);
                say(&r.message, if r.ok { "info" } else { "error" });
            }
            "run" if parts.get(1) == Some(&"--scheduled") => {
                // The host cron's headless fire: gates, then wait for the agent.
                let r = self.run_gated("scheduled", true).await;
                say(&r.message, if r.ok { "info" } else { "warning" });
            }
            "schedule" => {
                let cfg = self.cfg();
                let s = self.scheduler_json(&cfg).await;
                let mut lines = vec![format!("dream schedule: {}", Self::schedule_text(&cfg))];
                if let Some(c) = s.get("cron").and_then(Value::as_str) {
                    lines.push(format!(
                        "host cron job {} ({c}, local time)",
                        schedule::JOB_ID
                    ));
                }
                lines.push(if s["registered"] == true {
                    format!(
                        "registered{}",
                        s.get("nextRun")
                            .and_then(Value::as_str)
                            .map(|n| format!(" · next {n}"))
                            .unwrap_or_default()
                    )
                } else {
                    "not registered".to_owned()
                });
                if s["hostRunning"] != true {
                    lines.push("the Oino host is not reachable from here: scheduled dreams fire only while it runs".into());
                }
                if let Some(e) = s.get("error").and_then(Value::as_str) {
                    lines.push(format!("error: {e}"));
                }
                say(&lines.join("\n"), "info");
            }
            "run" => {
                let r = self.run_now().await;
                if r.ok {
                    say(
                        &format!("{}\nWatch it in the work tray (Dream tab).", r.message),
                        "info",
                    );
                } else {
                    say(&r.message, "warning");
                }
            }
            "stop" => {
                let r = self.stop(None);
                say(&r.message, if r.ok { "info" } else { "warning" });
            }
            _ => say(USAGE, "warning"),
        }
    }
}

/// `path.resolve(cwd, rel)`.
fn pathx_resolve(cwd: &str, rel: &str) -> String {
    let joined = if rel.starts_with('/') {
        rel.to_owned()
    } else {
        format!("{cwd}/{rel}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for s in joined.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    format!("/{}", parts.join("/"))
}

fn outcome_json(o: &Outcome) -> Value {
    json!({"ok": o.ok, "message": o.message})
}

impl Extension for Dream {
    #[allow(clippy::too_many_lines, reason = "registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        self.settings.declare(NAMESPACE, settings::defaults());
        self.settings.load(cx.settings());
        cx.settings_namespace(&settings::namespace().to_json());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.on_session_start(&cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.views.remove(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                let _ = ext.sync_schedule().await;
                ext.changed();
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, _cx| async move {
            if ev.get("timerId").and_then(Value::as_str) == Some(POLL_TIMER) {
                ext.changed();
            }
            Ok(())
        });
        cx.renderer(
            RendererTarget::CustomEntry,
            REPORT_CARD,
            |_ext, req, _cx| {
                let entry = req.get("data")?;
                let d = entry.get("data").unwrap_or(entry);
                let pending = d.get("pending").and_then(Value::as_u64)?;
                let card: Node = Block::default()
                    .borders(Borders::All)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(Color::Border))
                    .rail(Color::Harness)
                    .title(vec![Title::bottom(Line::from("Dream"))])
                    .child(
                        Layout::default().children([
                            Paragraph::new(Line::from(vec![
                                Span::from("▌ Dream report ready").bold(),
                                Span::from_string(format!(
                                    " {pending} proposal{} waiting",
                                    if pending == 1 { "" } else { "s" }
                                ))
                                .dim(),
                            ]))
                            .into(),
                            Paragraph::new(Line::from(
                                Span::from("/dream tray · /dream report · /dream approve <n>")
                                    .dim(),
                            ))
                            .into(),
                        ]),
                    )
                    .into();
                serde_json::to_value(card).ok()
            },
        );
        cx.service(fan::WORK_TRAY_TAB, |ext, payload, _caller, cx| async move {
            let req: TabRequest = serde_json::from_value(payload).unwrap_or_default();
            Ok(ext.tab(&req, &cx))
        });
        cx.service(
            point::WORK_TRAY_ACTION,
            |ext, payload, _caller, cx| async move {
                let a: Action = serde_json::from_value(payload)?;
                Ok(json!(ext.tray_action(&a, &cx).await))
            },
        );
        cx.service(point::DREAM_STATUS, |ext, _p, _caller, _cx| async move {
            Ok(ext.status_json().await)
        });
        cx.service("dream.runs", |ext, p, _caller, _cx| async move {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "small limits")]
            let limit = p.get("limit").and_then(Value::as_f64).filter(|l| *l > 0.0).map_or(50, |l| l as usize);
            Ok(json!({"runs": ext.runs().iter().take(limit).map(store::run_info).collect::<Vec<_>>()}))
        });
        cx.service(point::DREAM_RUN, |ext, _p, _caller, _cx| async move {
            Ok(outcome_json(&ext.run_now().await))
        });
        cx.service(point::DREAM_STOP, |ext, p, _caller, _cx| async move {
            Ok(outcome_json(&ext.stop(p.get("id").and_then(Value::as_str))))
        });
        cx.service("dream.detail", |ext, p, _caller, _cx| async move {
            let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
            Ok(ext
                .runs()
                .into_iter()
                .find(|r| r.id == id)
                .map_or(Value::Null, |r| {
                    let (steps, log) = store::run_trajectory(&r.staging, 200);
                    json!({
                        "id": r.id,
                        "run": store::run_info(&r),
                        "report": store::read_report(&r.staging),
                        "changes": store::read_changes(&r.staging),
                        "actions": store::read_actions(&r.staging),
                        "steps": steps,
                        "log": log,
                    })
                }))
        });
        for (name, approve) in [("dream.approve", true), ("dream.reject", false)] {
            cx.service(name, move |ext, p, _caller, cx| async move {
                let field = |k: &str| {
                    p.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                Ok(outcome_json(&ext.decide(
                    &field("runId"),
                    &field("proposalId"),
                    approve,
                    &cx.cwd().display().to_string(),
                )))
            });
        }
        cx.service("dream.dismiss", |ext, p, _caller, _cx| async move {
            Ok(outcome_json(&ext.dismiss(
                p.get("runId").and_then(Value::as_str).unwrap_or_default(),
            )))
        });

        cx.command(
            Command::<Self>::new("dream")
                .description("Dream (daily memory consolidation): /dream status|tray|report|run|stop|schedule|approve <n>|reject <n>")
                .run(|ext, args, cx| async move {
                    ext.command(&args, &cx).await;
                    Ok(())
                })
                .complete(|_ext, prefix, _cx| async move {
                    let q = prefix.trim().to_owned();
                    [
                        ("status", "status", "last run and what's scheduled"),
                        ("tray", "tray", "open the Dream tab"),
                        ("report", "report", "the latest dream report"),
                        ("run", "run", "dream now (in the background)"),
                        ("stop", "stop", "stop the running dream"),
                        ("schedule", "schedule", "the host schedule and the next dream"),
                        ("approve ", "approve <n>", "apply proposal n"),
                        ("reject ", "reject <n>", "drop proposal n"),
                    ]
                    .into_iter()
                    .filter(|(v, _, _)| v.trim().starts_with(&q))
                    .map(|(v, l, d)| CompletionItem {
                        value: v.into(),
                        label: Some(l.into()),
                        description: Some(d.into()),
                    })
                    .collect()
                }),
        );
        let _ = EXTENSION_ID;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Dream);

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
    fn prompt_and_names() {
        let projects = vec!["oino".to_owned(), "web".to_owned()];
        let p = dream_prompt(&PromptInputs {
            staging: "/s",
            memory_root: Some("/m"),
            projects: &projects,
            has_inventory: true,
            stale_days: 30.0,
        });
        assert!(p.contains("/s/digests/INDEX.json"));
        assert!(p.contains("/s/memory/before.json"));
        assert!(p.contains("for 30 days"));
        assert!(p.contains("Projects covered: oino, web."));
        assert!(p.contains("`merge_from`"), "consolidation");
        assert!(p.contains("Remove transient details"));
        assert!(p.contains("Find missed lessons"));
        assert!(p.contains("Prune stale memories"));
        assert!(p.contains("Never delete a `preference`"));
        assert!(p.contains("always pass `project`"));
        assert!(p.contains("[source: <session id>]"));
        assert!(p.contains("/s/memory-changes.json"));
        assert!(p.contains("/s/proposals/projects.json"));
        assert!(p.contains("under /m; read them, never edit them directly"));
        assert!(p.contains("/s/DREAM_REPORT.md exists with three sections"));
        let off = dream_prompt(&PromptInputs {
            staging: "/s",
            stale_days: 0.0,
            ..PromptInputs::default()
        });
        assert!(off.contains("inventory is unavailable"));
        assert!(off.contains("Pruning is off this run"));
        assert!(off.contains("Projects covered: (none"));
        assert_eq!(memory_project("/home/u/My Project!"), "my_project");
        assert_eq!(memory_project("/"), "unknown");
        assert_eq!(memory_project(r"C:\Users\u\Web App\"), "web_app");
        assert_eq!(
            pathx_resolve("/w/p", ".agents/skills/x"),
            "/w/p/.agents/skills/x"
        );
        let spec = Dream::agent_spec(&DreamSettings::default(), "/s", Some("/s/sessions/a.jsonl"));
        assert_eq!(spec["persistence"]["file"]["sessionDir"], "/s/sessions");
        assert_eq!(spec["seed"]["sessionFile"], "/s/sessions/a.jsonl");
        assert_eq!(spec["extensions"]["only"][0], "unipi.memory");
        assert!(spec.get("model").is_none());
    }
}
