//! `unipi.dream` — background reflection over past sessions (unipi `packages/dream`,
//! unipi-port §3.20).
//!
//! | unipi | here |
//! |---|---|
//! | due check on open (`minSessions`, `minGapHours`) | `session_start` (once per process, not in child sessions) |
//! | scrubbed struggle digests | [`digest`] over `/agent/sessions/--<cwd>--/*.jsonl` |
//! | detached `pi -p` child with craft-skill + memory tools, `timeout maxRuntimeMin` | nested agent (D8): `persistence: File` under the run's staging dir, extensions `only: [unipi.memory]`, aborted at `maxRuntimeMin`; interrupted runs resume once on the next start |
//! | report card entry `unipi-dream-report-card` + renderer | custom entry + OUS renderer |
//! | work-tray Dream tab (list/detail, o/s/r/a/x/d/1–9) | `work-tray.tab` / `work-tray.action` ([`tray`]) |
//! | `/unipi:dream [status\|tray\|report\|run\|stop\|approve <n>\|reject <n>]` | command |
//! | `unipi.dream.api` | services `dream.status`, `dream.run`, `dream.stop` (+ `dream.detail`, `dream.approve`, `dream.reject`, `dream.dismiss`) |
//! | craft-skill `check` before an approved skill lands | [`store::craft_check`] (Rust port); the bundled `craft-skill` skill keeps its scripts |
//! | `dream` settings | [`settings`] |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod digest;
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
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::services::{fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::topics;
use unipi_kit::tray::{Action, ActionReply, Counts, TabRequest, TrayTab};

use crate::settings::{DreamSettings, EXTENSION_ID, NAMESPACE};
use crate::store::{DreamState, Lock, Run, RunMeta, Status};
use crate::tray::View;

/// Report card entry type.
pub const REPORT_CARD: &str = "unipi-dream-report-card";
const POLL_TIMER: &str = "dream-poll";
const USAGE: &str = "usage: /unipi:dream status|tray|report|run|stop|approve <n>|reject <n>";

/// The dream prompt (unipi `dreamPrompt`); paths are host paths.
#[must_use]
pub fn dream_prompt(staging: &str, memory_root: Option<&str>) -> String {
    [
        "You are running a DREAM pass: an offline review of past coding-agent sessions. Nobody is watching; work only with files.".to_owned(),
        String::new(),
        "Inputs (read-only):".to_owned(),
        format!("- {staging}/digests/INDEX.json and the per-session files next to it: failed tool calls (call, errorHead = first line(s) of the error, errorTail = end of the error, recovery = the next successful call of the same tool) and user corrections."),
        String::new(),
        "Goal: make the next session take the shortest path. Find false paths: an error the agent hit, then recovered from with a different call. Group the same false path across sessions.".to_owned(),
        String::new(),
        "For each group, pick the cheapest fix that removes the false path:".to_owned(),
        "- seen in 1 session -> a memory lesson, or nothing if it was a one-off".to_owned(),
        "- seen in 2+ sessions and mechanical (fixed command shape, schema, banned pattern) -> propose an automated check or a one-line lesson keyed by the error text".to_owned(),
        "- seen in 2+ sessions and a multi-step procedure -> propose a skill (write it with the craft-skill skill, loaded for this run)".to_owned(),
        "Also: memory notes that are summaries of past work, duplicates, or contradicted by the digests -> merge, rewrite as a lesson, or delete.".to_owned(),
        String::new(),
        memory_root.map_or_else(
            || "Memory: edit the memory files directly under the memory directory given in the environment.".to_owned(),
            |m| format!("Memory: store, update and delete with the memory tools (memory_store / memory_delete) so the search index stays in sync. The memory files live under {m}."),
        ),
        String::new(),
        "Rules:".to_owned(),
        "- Every lesson and proposal cites at least one session id. No session id, no lesson.".to_owned(),
        "- A lesson is one line: `When <error text or situation>, <shortest path>. [source: <session id>]`".to_owned(),
        "- Never copy a secret, token, or password into any file.".to_owned(),
        format!("- Write skill and check proposals only under {staging}/proposals/, never anywhere else. Skills are crafted with the craft-skill skill (scaffold + check scripts)."),
        String::new(),
        format!("Done when {staging}/DREAM_REPORT.md exists with three sections: ## Memory edits (applied), ## Proposals (waiting for approval, each with its sources), ## Skipped (groups you judged one-off, with why)."),
    ]
    .join("\n")
}

/// unipi memory project-name rule.
#[must_use]
pub fn memory_project(cwd: &str) -> String {
    let base = cwd
        .trim_end_matches('/')
        .rsplit('/')
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
    checked: Cell<bool>,
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

impl Dream {
    fn cfg(&self) -> DreamSettings {
        settings::normalize(&self.settings.get(NAMESPACE).0)
    }

    fn root() -> PathBuf {
        unipi_kit::paths::global(&format!("runs/{}", host::workspace().id))
    }

    fn state_path() -> PathBuf {
        unipi_kit::paths::workspace("state.json")
    }

    fn sessions_dir(cwd: &str) -> PathBuf {
        PathBuf::from("/agent/sessions").join(digest::encode_session_dir_name(cwd))
    }

    fn alive(&self) -> impl Fn(&str) -> bool + '_ {
        |id| self.agents.borrow().contains_key(id)
    }

    fn runs(&self) -> Vec<Run> {
        let st = store::read_state(&Self::state_path());
        store::list_runs(&Self::root(), &st, timers::now_ms(), &self.alive())
    }

    fn changed(&self) {
        unipi_kit::bus::emit_raw(topics::WORK_TRAY_CHANGED, &json!({"tab": "dream"}));
        if self.agents.borrow().is_empty() {
            timers::cancel(POLL_TIMER);
        }
    }

    fn memory_root(cwd: &str) -> Option<String> {
        agent_dir().map(|a| {
            format!(
                "{a}/state/unipi.memory/global/memory/{}",
                memory_project(cwd)
            )
        })
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

    /// Spawns the agent for `staging` and watches it in the background.
    async fn launch(
        &'static self,
        cfg: &DreamSettings,
        staging: &Path,
        seed: Option<String>,
        prompt: &str,
    ) -> std::result::Result<(), String> {
        let id = staging
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let host = host_of(staging);
        let agent = Agent::spawn(&Self::agent_spec(cfg, &host, seed.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
        append_log(staging, &format!("dream agent {} started", agent.id()));
        agent.prompt(prompt).await.map_err(|e| e.to_string())?;
        let agent = Rc::new(agent);
        self.agents
            .borrow_mut()
            .insert(id.clone(), Rc::clone(&agent));
        timers::set(POLL_TIMER, Duration::from_secs(2), true);
        let max = Duration::from_secs_f64(cfg.max_runtime_min * 60.0);
        let staging = staging.to_path_buf();
        spawn(async move {
            let (code, error) = {
                let timeout = timers::sleep(max).fuse();
                let idle = agent.wait_idle().fuse();
                futures::pin_mut!(timeout, idle);
                futures::select! {
                    r = idle => match r {
                        Ok(()) => (0, None),
                        Err(e) => (1, Some(e.to_string())),
                    },
                    () = timeout => (124, None),
                }
            };
            if code == 124 {
                agent.abort();
            }
            if let Some(t) = agent.last_assistant_text() {
                append_log(&staging, &t);
            }
            if let Some(e) = &error {
                append_log(&staging, &format!("Error: {e}"));
            }
            let stopped = store::read_meta(&staging).is_some_and(|m| m.stopped);
            store::patch_meta(&staging, |m| {
                m.ended_at.get_or_insert(timers::now_ms());
                if !stopped {
                    m.exit_code = Some(code);
                    m.error.clone_from(&error);
                }
            });
            self.agents.borrow_mut().remove(&id);
            if let Ok(a) = Rc::try_unwrap(agent) {
                a.close(5_000).await;
            }
            let st_path = Self::state_path();
            let mut st = store::read_state(&st_path);
            if st.lock.as_ref().and_then(|l| l.run.as_deref()) == Some(id.as_str()) {
                st.lock = None;
                store::write_state(&st_path, &st);
            }
            self.changed();
        });
        Ok(())
    }

    /// unipi `startDream`: staging dir, digests, prompt, agent.
    async fn start(
        &'static self,
        cwd: &str,
        manual: bool,
    ) -> std::result::Result<(String, u32), String> {
        let cfg = self.cfg();
        let started = timers::now_ms();
        let staging = Self::root().join(format!("staging-{started}"));
        std::fs::create_dir_all(staging.join("proposals")).map_err(|e| e.to_string())?;
        let (sessions, events) =
            digest::digest_sessions(&Self::sessions_dir(cwd), &staging.join("digests"));
        let host = host_of(&staging);
        let prompt = dream_prompt(&host, Self::memory_root(cwd).as_deref());
        let _ = std::fs::write(staging.join("PROMPT.md"), &prompt);
        store::write_meta(
            &staging,
            &RunMeta {
                started_at: started,
                pid: -1,
                manual,
                sessions,
                events,
                ..RunMeta::default()
            },
        );
        if let Err(e) = self.launch(&cfg, &staging, None, &prompt).await {
            store::patch_meta(&staging, |m| {
                m.ended_at = Some(timers::now_ms());
                m.exit_code = Some(-1);
                m.error = Some(e.clone());
            });
            append_log(&staging, &format!("Error: {e}"));
            return Err(e);
        }
        let id = format!("staging-{started}");
        let mut st = store::read_state(&Self::state_path());
        st.last_run_at = started;
        st.lock = Some(Lock {
            pid: 0,
            at: started,
            run: Some(id.clone()),
        });
        st.shown_report = None;
        store::write_state(&Self::state_path(), &st);
        self.changed();
        Ok((id, sessions))
    }

    /// Resumes runs a previous process left unfinished (once each).
    async fn resume_interrupted(&'static self, cwd: &str) {
        let cfg = self.cfg();
        for run in self.runs().into_iter().filter(|r| r.interrupted) {
            let meta = store::read_meta(&run.staging).unwrap_or_default();
            let seed = store::resume_seed(&run.staging).map(|p| host_of(&p));
            if meta.resumes >= 1 || seed.is_none() {
                store::patch_meta(&run.staging, |m| {
                    m.ended_at = Some(timers::now_ms());
                    m.exit_code = Some(1);
                    m.error = Some("interrupted (Oino exited during the dream)".into());
                });
                continue;
            }
            store::patch_meta(&run.staging, |m| m.resumes += 1);
            let host = host_of(&run.staging);
            let prompt = format!(
                "The dream pass was interrupted. Continue from where you stopped.\n\n{}",
                dream_prompt(&host, Self::memory_root(cwd).as_deref())
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
        store::patch_meta(&run.staging, |m| {
            m.stopped = true;
            m.ended_at = Some(timers::now_ms());
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

    async fn run_now(&'static self, cwd: &str) -> Outcome {
        if self.runs().iter().any(|r| r.status == Status::Running) {
            return fail("A dream is already running.");
        }
        match self.start(cwd, true).await {
            Ok((_, sessions)) => ok(format!(
                "Dream running in the background ({sessions} sessions digested)."
            )),
            Err(_) => fail("Couldn't start the dream agent (see the dream state dir)."),
        }
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

    fn status_json(&self, cwd: &str) -> Value {
        let cfg = self.cfg();
        let st = store::read_state(&Self::state_path());
        let runs = self.runs();
        let mut v = json!({
            "v": 1,
            "enabled": cfg.enabled,
            "minSessions": cfg.min_sessions,
            "minGapHours": cfg.min_gap_hours,
            "lastRunAt": st.last_run_at,
            "running": runs.iter().any(|r| r.status == Status::Running),
            "runs": runs.iter().take(20).map(store::run_info).collect::<Vec<_>>(),
        });
        if cfg.enabled {
            let d = self.due(&st, &cfg, cwd);
            v["due"] = json!({"due": d.due, "reason": d.reason});
        }
        v
    }

    fn due(&self, st: &DreamState, cfg: &DreamSettings, cwd: &str) -> store::Due {
        let n = digest::count_new_sessions(&Self::sessions_dir(cwd), st.last_run_at);
        store::is_due(st, n, cfg, timers::now_ms(), &self.alive())
    }

    fn status_text(&self, cwd: &str) -> String {
        let cfg = self.cfg();
        let st = store::read_state(&Self::state_path());
        let runs = self.runs();
        let latest = runs.iter().find(|r| r.has_report);
        let mut lines = vec![
            if cfg.enabled {
                let d = self.due(&st, &cfg, cwd);
                format!(
                    "dream: on · {}",
                    if d.due {
                        "due at the next Oino open".to_owned()
                    } else {
                        format!("not due: {}", d.reason)
                    }
                )
            } else {
                "dream: off — turn on in /unipi:settings → Dream (manual /unipi:dream run still works)".to_owned()
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
                lines.push(
                    "approve with /unipi:dream approve <n> (or the work tray's Dream tab)".into(),
                );
            }
        }
        lines.join("\n")
    }

    async fn on_session_start(&'static self, cx: &Ctx) {
        self.settings.refresh();
        if cx.is_child() || self.checked.get() {
            return;
        }
        self.checked.set(true);
        let cwd = cx.cwd().display().to_string();
        self.resume_interrupted(&cwd).await;
        let p = Self::state_path();
        let st = store::read_state(&p);
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
        let cfg = self.cfg();
        if !cfg.enabled {
            return;
        }
        let due = self.due(&st, &cfg, &cwd);
        if !due.due {
            return;
        }
        if self.start(&cwd, false).await.is_ok() {
            let mut st = store::read_state(&p);
            st.sessions_seen = due.new_sessions;
            store::write_state(&p, &st);
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
            "run" => flash(self.run_now(&cwd).await.message),
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
            "status" => say(&self.status_text(&cwd), "info"),
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
                    say(&self.status_text(&cwd), "info");
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
                        &format!("usage: /unipi:dream {sub} <1-{}>", latest.proposals.len()),
                        "warning",
                    );
                };
                let r = self.decide(&latest.id, &p.id, sub == "approve", &cwd);
                say(&r.message, if r.ok { "info" } else { "error" });
            }
            "run" => {
                let r = self.run_now(&cwd).await;
                if r.ok {
                    say(
                        &format!("{}\nWatch it in the work tray (↓ → Dream).", r.message),
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
        cx.renderer(RendererTarget::CustomEntry, REPORT_CARD, |_ext, req, _cx| {
            let entry = req.get("data")?;
            let d = entry.get("data").unwrap_or(entry);
            let pending = d.get("pending").and_then(Value::as_u64)?;
            Some(json!({"t": "card", "rail": "harness", "source": "Dream", "body": {"t": "stack", "direction": "vertical", "children": [
                {"t": "text", "value": [{"text": "▌ Dream report ready", "bold": true}, {"text": format!(" {pending} proposal{} waiting", if pending == 1 { "" } else { "s" }), "dim": true}]},
                {"t": "text", "value": [{"text": "/unipi:dream tray · /unipi:dream report · /unipi:dream approve <n>", "dim": true}]},
            ]}}))
        });
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
        cx.service(point::DREAM_STATUS, |ext, _p, _caller, cx| async move {
            Ok(ext.status_json(&cx.cwd().display().to_string()))
        });
        cx.service(point::DREAM_RUN, |ext, _p, _caller, cx| async move {
            Ok(outcome_json(
                &ext.run_now(&cx.cwd().display().to_string()).await,
            ))
        });
        cx.service(point::DREAM_STOP, |ext, p, _caller, _cx| async move {
            Ok(outcome_json(&ext.stop(p.get("id").and_then(Value::as_str))))
        });
        cx.service("dream.detail", |ext, p, _caller, _cx| async move {
            let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
            Ok(ext.runs().into_iter().find(|r| r.id == id).map_or(Value::Null, |r| {
                let (steps, log) = store::run_trajectory(&r.staging, 200);
                json!({"id": r.id, "report": store::read_report(&r.staging), "steps": steps, "log": log})
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
            Command::<Self>::new("unipi:dream")
                .description("Dream (background memory consolidation): /unipi:dream status|tray|report|run|stop|approve <n>|reject <n>")
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
        let p = dream_prompt("/s", Some("/m"));
        assert!(p.contains("/s/digests/INDEX.json"));
        assert!(p.contains("memory_store / memory_delete"));
        assert!(p.contains("/s/DREAM_REPORT.md exists with three sections"));
        assert!(dream_prompt("/s", None).contains("edit the memory files directly"));
        assert_eq!(memory_project("/home/u/My Project!"), "my_project");
        assert_eq!(memory_project("/"), "unknown");
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
