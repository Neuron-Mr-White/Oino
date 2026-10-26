//! `unipi.kanboard` — the unipi 3.0.0 kanboard package as an Oino component (unipi-port
//! §3.16). The board is written only by the native `oino-kanboard` binary (crate
//! `crates/oino-kanboard`); this component never edits task files.
//!
//! | unipi | here |
//! |---|---|
//! | `/unipi:kanboard [open\|close\|onboard\|status\|show\|doctor]`, `-add`, `-do`, `-autowork` | commands on [`Kanboard`] (see [`args`] for texts and parsing) |
//! | runners open / stop-daemon / rotate-token | hidden commands [`settings::ACTION_OPEN`] … |
//! | bash write guard (`tool_call`) | [`guard`] + `tool_call` handler (also exports the session env, see below) |
//! | arbiter monitor (claims 50 / autowork 40) | [`monitor`] behind `propose-nudge` |
//! | evidence contributor, compaction context, move handler, `captureToBacklog` | services `evidence.contribute`, `compaction-context.provide`, `move.handle`, `kanboard.capture` |
//! | R1 reminders, badges, notices, progress bar | [`reminders`], [`render`] (+ OUS renderers) |
//! | in-chat board (message renderer) | [`render::show_node`] |
//! | web board | [`panel`] (`sidePane`, refreshed from the daemon's SSE) |
//! | `process.env.UNIPI_KANBOARD_*` for the agent's bash | `tool_call` prepends `export OINO_KANBOARD_SESSION=… PID=… PROJECT=… MAX_SESSIONS=…` to commands that call the binary |
//! | sticky `KANBOARD_STATUS` | `unipi:kanboard:status` |

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "fallible fns return the CLI's or the contract's error, documented on the error types"
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

pub mod args;
pub mod cli;
pub mod guard;
pub mod monitor;
pub mod panel;
pub mod reminders;
pub mod render;
pub mod settings;
pub mod shapes;
pub mod shell;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use oino_extension::command::CompletionItem;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::services::{
    CompactionContext, EvidenceContribution, MoveReport, MoveRequest, fanout as fan, point,
};
use unipi_kit::settings::Store;
use unipi_kit::topics::{self, KanboardStatus, LhOwnerChanged, LhState, ModuleReady, SkillsReveal};

use crate::args::{AddCompletion, DOCTOR_CUSTOM_TYPE, HELP, HELP_CUSTOM_TYPE, SHOW_CUSTOM_TYPE};
use crate::cli::CliError;
use crate::guard::WriteGuard;
use crate::monitor::{Monitor, Notice, OwnerStatus, SettleInfo};
use crate::reminders::Tracker;
use crate::render::{BADGE_ENTRY, NOTICE_ENTRY, PROGRESS_ENTRY};
use crate::settings::{
    ACTION_OPEN, ACTION_ROTATE_TOKEN, ACTION_STOP_DAEMON, KanboardSettings, NAMESPACE,
};
use crate::shapes::Task;

/// The skill utility reveals on `-do`/autowork.
pub const KANBOARD_SKILL: &str = "kanboard";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Per-session state.
#[derive(Default)]
struct SessionState {
    guard: WriteGuard,
    monitor: Monitor,
    tracker: Tracker,
    notices: Vec<Notice>,
    tool_calls: u32,
    settle: SettleInfo,
    panel_open: bool,
    panel_gen: u64,
    selected: Option<String>,
    cache: Option<(i64, Vec<Task>)>,
    live: bool,
}

/// The extension.
#[derive(Default)]
pub struct Kanboard {
    settings: Store,
    sessions: SessionMap<SessionState>,
    /// Absolute path the binary reported (`status.binary`).
    binary_path: RefCell<Option<String>>,
    /// Why the binary cannot run (`None` = fine or not probed yet).
    unavailable: RefCell<Option<String>>,
    /// Project slug per cwd.
    slugs: RefCell<HashMap<String, String>>,
    /// Last non-empty `--actor agent --project` prefix (survives re-attach gaps).
    last_prefix: RefCell<Option<String>>,
    /// Running as kanboard's own child (`OINO_KANBOARD_CHILD`, the daemon's summarizer).
    own_child: Cell<bool>,
    /// This machine's host name (`status.hostname`).
    host_name: RefCell<Option<String>>,
    seq: Cell<u64>,
}

fn wit_spawn(fut: impl Future<Output = ()> + 'static) {
    #[cfg(target_arch = "wasm32")]
    oino_extension::wit_bindgen::spawn_local(fut);
    #[cfg(not(target_arch = "wasm32"))]
    drop(fut);
}

fn notify(text: &str, level: &str) {
    let _ = ui::notify(text, level);
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `export …` line that hands the session identity to the agent's own CLI calls.
#[must_use]
pub fn env_prefix(tool: &str, vars: &[(&str, String)]) -> String {
    let quote = |v: &str| format!("'{}'", v.replace('\'', "'\\''"));
    if tool == "powershell" {
        let sets: Vec<String> = vars
            .iter()
            .map(|(k, v)| format!("$env:{k}='{}'", v.replace('\'', "''")))
            .collect();
        return format!("{}; ", sets.join("; "));
    }
    let sets: Vec<String> = vars
        .iter()
        .map(|(k, v)| format!("{k}={}", quote(v)))
        .collect();
    format!("export {}\n", sets.join(" "))
}

impl Kanboard {
    fn config(&self) -> KanboardSettings {
        let values = self.settings.get(NAMESPACE);
        let stored = oino_extension::control::settings().ok().and_then(|s| {
            s.pointer(&format!("/extensionSettings/{NAMESPACE}"))
                .cloned()
        });
        KanboardSettings::from_values(&values, stored.as_ref())
    }

    fn session_tag(cx: &Ctx) -> String {
        format!("oino-{}", cx.session_id())
    }

    fn is_child(&self, cx: &Ctx) -> bool {
        cx.is_child() || self.own_child.get()
    }

    fn binary(&self) -> String {
        if let Ok(Some(path)) = host::env("OINO_KANBOARD_BIN")
            && !path.trim().is_empty()
        {
            return path.trim().to_owned();
        }
        self.binary_path
            .borrow()
            .clone()
            .unwrap_or_else(|| shell::BINARY.to_owned())
    }

    fn next_id(&self, what: &str) -> String {
        self.seq.set(self.seq.get() + 1);
        format!("kanboard-{what}-{}-{}", timers::now_ms(), self.seq.get())
    }

    fn base_env(&self, cx: &Ctx, slug: Option<&str>) -> Vec<(String, String)> {
        let cfg = self.config();
        let mut env = vec![
            ("OINO_KANBOARD_SESSION".to_owned(), Self::session_tag(cx)),
            (
                "OINO_KANBOARD_MAX_SESSIONS".to_owned(),
                cfg.max_sessions.to_string(),
            ),
            (
                "OINO_KANBOARD_CHAIN_GATE".to_owned(),
                cfg.chain_gate.as_str().to_owned(),
            ),
        ];
        if let Some(slug) = slug {
            env.push(("OINO_KANBOARD_PROJECT".to_owned(), slug.to_owned()));
        }
        env
    }

    /// Runs the binary for `cx` (project + session env, cwd = session cwd).
    async fn run(
        &self,
        cx: &Ctx,
        args: &[&str],
        slug: Option<&str>,
        json: bool,
    ) -> Result<Value, CliError> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let env = self.base_env(cx, slug);
        let cwd = cx.cwd().to_string_lossy().into_owned();
        let result = cli::run(&self.binary(), &args, &env, Some(&cwd), json, None).await;
        if let Err(e) = &result
            && e.kind == "io"
            && (e.message.contains("permission denied")
                || e.message.contains("No such file")
                || e.message.contains("not found")
                || e.message.contains("failed to spawn"))
        {
            let info = host::info();
            *self.unavailable.borrow_mut() =
                Some(cli::unavailable_message(&info.platform, &info.arch));
        }
        result
    }

    /// Probes the binary (`status`), caching its absolute path and the host name.
    async fn attach(&self, cx: &Ctx) -> Result<shapes::Status, String> {
        match self.run(cx, &["status"], None, true).await {
            Ok(v) => {
                let status = shapes::as_status(&v).map_err(|e| e.to_string())?;
                if let Some(b) = &status.binary {
                    *self.binary_path.borrow_mut() = Some(b.clone());
                }
                self.host_name.borrow_mut().clone_from(&status.hostname);
                *self.unavailable.borrow_mut() = None;
                Ok(status)
            }
            Err(e) => {
                let why = self.unavailable.borrow().clone().unwrap_or(e.message);
                Err(why)
            }
        }
    }

    async fn unavailable(&self, cx: &Ctx) -> Option<String> {
        if self.binary_path.borrow().is_some() {
            return None;
        }
        self.attach(cx).await.err()
    }

    /// The project slug for this cwd: settings `slug`, else the cache, else the project the
    /// binary resolves for the cwd (git root) — `None` when not onboarded.
    async fn slug(&self, cx: &Ctx) -> Option<String> {
        if let Some(slug) = self
            .settings
            .get(NAMESPACE)
            .get("slug")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Some(slug.to_owned());
        }
        let cwd = cx.cwd().to_string_lossy().into_owned();
        if let Some(slug) = self.slugs.borrow().get(&cwd) {
            return Some(slug.clone());
        }
        let v = self.run(cx, &["project", "show"], None, true).await.ok()?;
        let (project, _) = shapes::as_project_detail(&v).ok()?;
        self.slugs.borrow_mut().insert(cwd, project.slug.clone());
        Some(project.slug)
    }

    async fn cli_prefix(&self, cx: &Ctx) -> Option<String> {
        let slug = self.slug(cx).await;
        let current = slug.map(|s| format!("{} --actor agent --project {s}", self.binary()));
        if let Some(c) = &current {
            *self.last_prefix.borrow_mut() = Some(c.clone());
        }
        current.or_else(|| self.last_prefix.borrow().clone())
    }

    async fn list(&self, cx: &Ctx, extra: &[&str]) -> Result<Vec<Task>, String> {
        let Some(slug) = self.slug(cx).await else {
            return Ok(Vec::new());
        };
        let mut args = vec!["list"];
        args.extend_from_slice(extra);
        let v = self
            .run(cx, &args, Some(&slug), true)
            .await
            .map_err(|e| e.message)?;
        shapes::as_task_list(&v)
            .map(|(t, _)| t)
            .map_err(|e| e.to_string())
    }

    async fn own_claims(&self, cx: &Ctx) -> Vec<Task> {
        let tag = Self::session_tag(cx);
        self.list(cx, &[])
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.claimed_by(&tag))
            .collect()
    }

    async fn refresh_holder(&self, cx: &Ctx) {
        let claims: Vec<String> = self
            .own_claims(cx)
            .await
            .into_iter()
            .map(|t| t.id)
            .collect();
        let autowork = self
            .sessions
            .with(cx.session_id(), |s| s.guard.remaining().autowork);
        let cli = self.cli_prefix(cx).await;
        unipi_kit::bus::emit(&KanboardStatus {
            claims,
            autowork,
            cli,
        });
    }

    fn reveal_skill() {
        unipi_kit::bus::emit(&SkillsReveal {
            names: vec![KANBOARD_SKILL.to_owned()],
        });
    }

    async fn send_harness_user(text: &str, title: &str, synopsis: &str) {
        let busy = !oino_extension::control::is_idle();
        let opts = oino_extension::wit::session::UserMessageOptions {
            deliver_as: busy.then_some(oino_extension::session::Delivery::FollowUp),
            expand_prompt_templates: false,
            provenance: Some(oino_extension::session::Provenance {
                source: "Kanboard".into(),
                title: Some(title.to_owned()),
                synopsis: Some(synopsis.to_owned()),
                severity: None,
            }),
        };
        let _ = oino_extension::wit::session::send_user_message(
            Value::String(text.to_owned()).to_string(),
            opts,
        )
        .await;
    }

    async fn display_message(custom_type: &str, content: String, details: Option<Value>) {
        let msg = oino_extension::session::CustomMessage {
            custom_type: custom_type.to_owned(),
            content: Value::String(content),
            display: true,
            details,
        };
        let _ = session::send_message(&msg, oino_extension::session::SendOptions::default()).await;
    }

    // ── commands ────────────────────────────────────────────────────────

    async fn onboard(&self, cx: &Ctx) -> Option<String> {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return None;
        }
        match self
            .run(cx, &["project", "add"], None, true)
            .await
            .map_err(|e| e.message)
            .and_then(|v| shapes::as_project("project add", &v).map_err(|e| e.to_string()))
        {
            Ok(project) => {
                self.slugs.borrow_mut().insert(
                    cx.cwd().to_string_lossy().into_owned(),
                    project.slug.clone(),
                );
                // unipi `setSettings("kanboard", {slug}, "project")`; untrusted projects keep
                // the in-memory cache only.
                let _ = self.settings.write(
                    NAMESPACE,
                    "slug",
                    &json!(project.slug),
                    unipi_kit::settings::WriteScope::Project,
                );
                notify(
                    &[
                        format!("kanboard: {} ({}) registered", project.name, project.slug),
                        "  /unipi:kanboard-add <title>     — capture a task into Backlog".into(),
                        "  /unipi:kanboard-do <request>    — let the agent use the board for a turn".into(),
                        "  /unipi:kanboard-autowork start  — work ready tasks one by one".into(),
                        "  /unipi:kanboard open            — open the board panel".into(),
                    ]
                    .join("\n"),
                    "info",
                );
                Some(project.slug)
            }
            Err(e) => {
                notify(&format!("kanboard: onboarding failed — {e}"), "error");
                None
            }
        }
    }

    async fn ensure_onboarded(&self, cx: &Ctx) -> Option<String> {
        if let Some(slug) = self.slug(cx).await {
            return Some(slug);
        }
        let ok = ui::confirm(
            "Kanboard is not set up here",
            "Register this project on the board first?",
        )
        .await
        .unwrap_or(false);
        if !ok {
            return None;
        }
        self.onboard(cx).await
    }

    async fn add(&self, cx: &Ctx, text: &str) {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        let parsed = args::parse_add_args(text);
        if let Some(e) = parsed.error {
            notify(
                &format!(
                    "kanboard: {e} — /unipi:kanboard-add [-p 1-5] [--after ID] [--status backlog|todo] <title>"
                ),
                "warning",
            );
            return;
        }
        if parsed.title.is_empty() && parsed.description.trim().is_empty() {
            notify(
                "kanboard: add needs a title or a description — /unipi:kanboard-add <title> (lines below are the description)",
                "warning",
            );
            return;
        }
        let Some(slug) = self.ensure_onboarded(cx).await else {
            return;
        };
        match self
            .add_task(
                cx,
                &slug,
                &parsed.title,
                &parsed.flags,
                &parsed.description,
                &parsed.attaches,
            )
            .await
        {
            Ok(task) => {
                let lane = if task.status == "todo" {
                    "Todo"
                } else {
                    "Backlog"
                };
                let n = task.attachment_count();
                let att = if n > 0 {
                    format!(" ({n} attachment{})", if n == 1 { "" } else { "s" })
                } else {
                    String::new()
                };
                notify(&format!("✓ {} added to {lane}{att}", task.id), "info");
            }
            Err(e) => notify(&format!("kanboard: could not add the task — {e}"), "error"),
        }
    }

    /// `add` with the body passed through a temp file (no re-escaping through argv).
    async fn add_task(
        &self,
        cx: &Ctx,
        slug: &str,
        title: &str,
        flags: &[String],
        body: &str,
        attaches: &[String],
    ) -> Result<Task, String> {
        let mut argv: Vec<String> = vec!["add".into()];
        if !title.is_empty() {
            argv.push(title.to_owned());
        }
        argv.extend(flags.iter().cloned());
        let mut tmp = None;
        if !body.is_empty() {
            let guest = unipi_kit::paths::tmp(&format!("kb-add-{}.md", self.next_id("body")));
            let host = unipi_kit::paths::ensure_parent(&guest)
                .ok()
                .and_then(|()| std::fs::write(&guest, body).ok())
                .and_then(|()| unipi_kit::paths::host_path(&guest));
            match host {
                Some(h) => {
                    argv.extend(["--body-file".into(), h.to_string_lossy().into_owned()]);
                    tmp = Some(guest);
                }
                None => argv.push(format!("--body={body}")),
            }
        }
        for a in attaches {
            argv.extend(["--attach-existing".into(), a.clone()]);
        }
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let result = self.run(cx, &refs, Some(slug), true).await;
        if let Some(t) = tmp {
            unipi_kit::paths::try_delete(&t);
        }
        let v = result.map_err(|e| e.message)?;
        shapes::as_task("add", &v).map_err(|e| e.to_string())
    }

    async fn capture(&self, cx: &Ctx, payload: &Value) -> Value {
        if let Some(why) = self.unavailable(cx).await {
            return json!({"ok": false, "reason": why});
        }
        let Some(slug) = self.slug(cx).await else {
            return json!({"ok": false, "reason": "kanboard is not set up here — run /unipi:kanboard"});
        };
        let text = payload
            .get("text")
            .or_else(|| payload.get("body"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let title = s(payload, "title");
        if text.is_empty() && title.trim().is_empty() {
            return json!({"ok": false, "reason": "nothing to add"});
        }
        let mut flags = vec!["--status".to_owned(), "backlog".to_owned()];
        if let Some(p) = payload
            .get("priority")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
        {
            flags.extend(["--priority".to_owned(), p.to_owned()]);
        }
        let attaches = args::detect_file_paths(&text);
        match self
            .add_task(cx, &slug, title.trim(), &flags, &text, &attaches)
            .await
        {
            Ok(task) => json!({"ok": true, "id": task.id, "attachments": task.attachment_count()}),
            Err(reason) => json!({"ok": false, "reason": reason}),
        }
    }

    async fn daemon(&self, cx: &Ctx) -> Option<shapes::DaemonInfo> {
        let status = self.attach(cx).await.ok()?;
        status.daemon.filter(|_| status.alive)
    }

    async fn healthy(port: u16, token: Option<&str>) -> bool {
        let mut req = http::get(&format!("http://127.0.0.1:{port}/api/health")).timeout_ms(700);
        if let Some(t) = token {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        match req.send().await {
            Ok(resp) if resp.status() == 200 => {
                resp.json::<Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("ok").and_then(Value::as_bool))
                    == Some(true)
            }
            _ => false,
        }
    }

    /// Ensures the daemon runs on the requested binding; a different binding restarts it.
    async fn ensure_daemon(&self, cx: &Ctx, host: &str, port: u16) -> Option<(u16, bool)> {
        let cfg = self.config();
        let mut restarted = false;
        if let Some(existing) = self.daemon(cx).await
            && Self::healthy(existing.port, existing.token.as_deref()).await
        {
            let same = existing.host.as_deref().unwrap_or("127.0.0.1") == host
                && (port == 0 || existing.port == port);
            if same {
                let wants_token = cfg.require_auth || !args::is_loopback_host(host);
                let has_token = existing.token.as_deref().is_some_and(|t| !t.is_empty());
                if wants_token != has_token {
                    let mode = if has_token {
                        "token-gated"
                    } else {
                        "open (no token)"
                    };
                    notify(
                        &format!(
                            "kanboard: the board is running {mode} on {}:{} — /unipi:kanboard close then open to apply",
                            existing.host.as_deref().unwrap_or("127.0.0.1"),
                            existing.port
                        ),
                        "warning",
                    );
                }
                return Some((existing.port, false));
            }
            let _ = self.run(cx, &["stop"], None, true).await;
            restarted = true;
        }
        let mut argv = vec![
            "serve".to_owned(),
            "--host".into(),
            host.to_owned(),
            "--port".into(),
            port.to_string(),
            "--idle-min".into(),
            cfg.idle_min.to_string(),
        ];
        if cfg.require_auth {
            argv.push("--require-auth".into());
        }
        if cfg.keep_token {
            argv.push("--keep-token".into());
        }
        let mut spec = process::spec(&self.binary(), &[]);
        spec.args = argv;
        spec.stdin = process::StdioMode::Null;
        spec.stdout = process::OutputTarget::Null;
        spec.stderr = process::OutputTarget::Null;
        spec.detached = true;
        spec.env = self.base_env(cx, None);
        spec.env.push(("OINO_KANBOARD_ACTOR".into(), "user".into()));
        if let Err(e) = process::Child::spawn(&spec) {
            notify(
                &format!("kanboard: could not start the daemon — {e}"),
                "error",
            );
            return None;
        }
        for _ in 0..20 {
            timers::sleep(std::time::Duration::from_millis(150)).await;
            if let Some(info) = self.daemon(cx).await
                && Self::healthy(info.port, info.token.as_deref()).await
            {
                return Some((info.port, restarted));
            }
        }
        notify("kanboard: the daemon did not answer within 3s", "warning");
        None
    }

    async fn open(&'static self, cx: &Ctx, rest: &str) {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        let cfg = self.config();
        let flags = args::parse_open_args(rest, &cfg.host, cfg.port);
        if !flags.unknown.is_empty() {
            notify(
                &format!(
                    "kanboard: ignoring unknown option(s) {}",
                    flags.unknown.join(" ")
                ),
                "warning",
            );
        }
        let host = match self.resolve_host(cx, &flags.host).await {
            Ok(h) => h,
            Err(e) => {
                notify(&format!("kanboard: {e}"), "error");
                return;
            }
        };
        let Some((port, restarted)) = self.ensure_daemon(cx, &host, flags.port).await else {
            return;
        };
        if restarted {
            notify(
                &format!("kanboard: restarted kanboard on {host}:{port}"),
                "info",
            );
        }
        self.sync_runtime(cx).await;
        let info = self.daemon(cx).await;
        let tailscale = if host == "0.0.0.0" {
            self.resolve_host(cx, "tailscale").await.ok()
        } else {
            None
        };
        let slug = self.slug(cx).await;
        let hostname = self.host_name.borrow().clone();
        let (urls, warnings) = args::format_board_urls(
            &host,
            port,
            slug.as_deref(),
            info.as_ref().and_then(|i| i.token.as_deref()),
            hostname.as_deref(),
            tailscale.as_deref(),
        );
        for url in urls {
            notify(&format!("kanboard: {url}"), "info");
        }
        for w in warnings {
            notify(&format!("kanboard: ⚠ {w}"), "warning");
        }
        if cx.has_ui() {
            self.open_panel(cx).await;
        }
    }

    async fn resolve_host(&self, cx: &Ctx, host: &str) -> Result<String, String> {
        if host.trim() != "tailscale" {
            return Ok(host.trim().to_owned());
        }
        let cwd = cx.cwd().to_string_lossy().into_owned();
        match process::exec("tailscale", &["ip", "-4"], Some(&cwd), Some(5_000)).await {
            Ok(out) if out.code == Some(0) => args::first_line(&out.stdout).ok_or_else(|| {
                "tailscale ip -4 returned nothing — is this machine on a tailnet?".to_owned()
            }),
            Ok(out) => Err(format!(
                "tailscale is not available ({}) — install it or pass --host <addr>",
                out.stderr.trim()
            )),
            Err(e) => Err(format!(
                "tailscale is not available ({e}) — install it or pass --host <addr>"
            )),
        }
    }

    /// Tells the daemon how to run Oino for summaries (`piCommand = ["oino"]`; the binary
    /// prefers the `oino` next to itself). Skipped inside kanboard's own children.
    async fn sync_runtime(&self, cx: &Ctx) {
        if self.own_child.get() {
            return;
        }
        let _ = self
            .run(
                cx,
                &["settings", "set", "pi-command", "[\"oino\"]"],
                None,
                true,
            )
            .await;
    }

    async fn stop_daemon(&self, cx: &Ctx) {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        match self
            .run(cx, &["stop"], None, true)
            .await
            .map_err(|e| e.message)
            .and_then(|v| shapes::as_stop(&v).map_err(|e| e.to_string()))
        {
            Ok((true, pid, _)) => notify(
                &format!("kanboard daemon stopped (pid {})", pid.unwrap_or(0)),
                "info",
            ),
            Ok((false, _, reason)) => notify(
                &format!(
                    "kanboard: {}",
                    reason.unwrap_or_else(|| "not running".into())
                ),
                "info",
            ),
            Err(e) => notify(&format!("kanboard: {e}"), "error"),
        }
    }

    async fn rotate_token(&self, cx: &Ctx) {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        match self.run(cx, &["rotate-token"], None, true).await {
            Ok(_) => notify(
                "kanboard: new token on the next daemon start — /unipi:kanboard close, then open",
                "info",
            ),
            Err(e) => notify(&format!("kanboard: rotate-token failed — {e}"), "error"),
        }
    }

    async fn status(&self, cx: &Ctx) {
        let mut lines = Vec::new();
        match self.daemon(cx).await {
            Some(d) if Self::healthy(d.port, d.token.as_deref()).await => lines.push(format!(
                "daemon: pid {} · http://{}:{} · v{}",
                d.pid,
                d.host.as_deref().unwrap_or("127.0.0.1"),
                d.port,
                d.version
            )),
            _ => lines.push("daemon: not running".into()),
        }
        match self.slug(cx).await {
            Some(slug) => match self.list(cx, &["--all"]).await {
                Ok(tasks) => {
                    let mut counts: Vec<(String, usize)> = Vec::new();
                    for t in &tasks {
                        match counts.iter_mut().find(|(s, _)| *s == t.status) {
                            Some((_, n)) => *n += 1,
                            None => counts.push((t.status.clone(), 1)),
                        }
                    }
                    let summary = if counts.is_empty() {
                        "empty".to_owned()
                    } else {
                        counts
                            .iter()
                            .map(|(s, n)| format!("{s} {n}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    lines.push(format!(
                        "project: {slug} · {} tasks ({summary})",
                        tasks.len()
                    ));
                    if let Ok(v) = self.run(cx, &["list", "--all"], Some(&slug), true).await
                        && let Ok((_, problems)) = shapes::as_task_list(&v)
                        && !problems.is_empty()
                    {
                        lines.push(format!(
                            "⚠ {} task file(s) need repair — oino-kanboard validate --fix",
                            problems.len()
                        ));
                    }
                    if let Some(bar) = render::board_progress(&tasks, &slug) {
                        let _ = session::append_entry(PROGRESS_ENTRY, Some(&bar));
                    }
                    let running: Vec<&Task> =
                        tasks.iter().filter(|t| t.status == "in_progress").collect();
                    if !running.is_empty() {
                        lines.push("claims:".into());
                        for t in running {
                            let run = t.run.clone().unwrap_or_default();
                            lines.push(format!(
                                "  {} — session {} pid {} on {} ({})",
                                t.id,
                                run.session.unwrap_or_else(|| "?".into()),
                                run.pid.map_or_else(|| "?".into(), |p| p.to_string()),
                                run.host.unwrap_or_else(|| "?".into()),
                                t.staleness.clone().unwrap_or_else(|| "?".into())
                            ));
                        }
                    }
                }
                Err(_) => lines.push(format!("project: {slug} (unreadable board)")),
            },
            None => lines.push("project: not registered here — /unipi:kanboard onboard".into()),
        }
        notify(&lines.join("\n"), "info");
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one check per line of the report, in unipi's order"
    )]
    async fn doctor(&self, cx: &Ctx) {
        let check = |ok: Option<bool>, label: String| {
            format!(
                "{} {label}",
                match ok {
                    None => "–",
                    Some(true) => "✓",
                    Some(false) => "✗",
                }
            )
        };
        let mut lines = vec!["kanboard doctor".to_owned()];
        let attached = self.attach(cx).await;
        let source = if host::env("OINO_KANBOARD_BIN").ok().flatten().is_some() {
            "env"
        } else {
            "install"
        };
        lines.push(match &attached {
            Ok(_) => check(Some(true), format!("binary: {} ({source})", self.binary())),
            Err(why) => check(Some(false), format!("binary: {why}")),
        });
        let daemon = attached.as_ref().ok().and_then(|s| s.daemon.clone());
        let alive = match &daemon {
            Some(d) => Self::healthy(d.port, d.token.as_deref()).await,
            None => false,
        };
        lines.push(check(
            Some(alive),
            daemon.as_ref().map_or_else(
                || "daemon: not running".to_owned(),
                |d| {
                    format!(
                        "daemon: http://{}:{} alive (pid {})",
                        d.host.as_deref().unwrap_or("127.0.0.1"),
                        d.port,
                        d.pid
                    )
                },
            ),
        ));
        if let Some(d) = &daemon {
            let bind = d.host.clone().unwrap_or_else(|| "127.0.0.1".into());
            let loopback = args::is_loopback_host(&bind);
            let host = if loopback {
                "127.0.0.1".to_owned()
            } else {
                self.host_name
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| "host".into())
            };
            let mode = if loopback {
                "(loopback — local only)".to_owned()
            } else {
                format!("(remote — token-gated, e.g. http://{host}:{}?t=…)", d.port)
            };
            lines.push(check(Some(true), format!("bind: {bind}:{} {mode}", d.port)));
        }
        let slug = self.slug(cx).await;
        match &slug {
            None => lines.push(check(
                Some(false),
                "project: not registered for this folder — /unipi:kanboard onboard".into(),
            )),
            Some(slug) if attached.is_ok() => {
                match self.run(cx, &["project", "show"], Some(slug), true).await {
                    Ok(v) => {
                        let name = v
                            .pointer("/project/name")
                            .and_then(Value::as_str)
                            .unwrap_or(slug)
                            .to_owned();
                        let total = v
                            .get("total")
                            .map_or_else(|| "?".into(), ToString::to_string);
                        lines.push(check(
                            Some(true),
                            format!("project: {name} ({slug}) · {total} tasks"),
                        ));
                    }
                    Err(e) => lines.push(check(
                        Some(false),
                        format!("project: {slug} unreadable — {e}"),
                    )),
                }
            }
            Some(_) => {}
        }
        match self.run(cx, &["settings", "show"], None, true).await {
            Ok(v) => {
                let argv: Vec<String> = v
                    .get("piCommand")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if argv.is_empty() {
                    lines.push(check(
                        None,
                        "summarize via oino: not configured (open the board from Oino once)".into(),
                    ));
                } else {
                    lines.push(check(
                        Some(true),
                        format!("summarize via oino: {}", argv.join(" ")),
                    ));
                    let model = s(&v, "summaryModel");
                    if model.trim().is_empty() {
                        lines.push(check(Some(true), "summary model: oino default".into()));
                    } else {
                        let listed = v
                            .get("models")
                            .and_then(Value::as_array)
                            .is_some_and(|m| m.iter().any(|x| x.as_str() == Some(model.trim())));
                        lines.push(check(
                            Some(listed),
                            format!(
                                "summary model: {model}{}",
                                if listed {
                                    ""
                                } else {
                                    " — not in the reported model list"
                                }
                            ),
                        ));
                    }
                }
            }
            Err(_) => lines.push(check(
                None,
                "summarize via oino: not configured (open the board from Oino once)".into(),
            )),
        }
        if let Ok(settings) = oino_extension::control::settings() {
            let mode = settings
                .pointer("/extensionSettings/permission/mode")
                .and_then(Value::as_str)
                .unwrap_or("auto");
            lines.push(check(Some(true), format!("permission mode: {mode}")));
        }
        if let (Some(slug), true) = (&slug, attached.is_ok()) {
            match self.run(cx, &["reap", "--dry-run"], Some(slug), true).await {
                Ok(v) => {
                    let ids = |k: &str| {
                        v.get(k)
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(ToOwned::to_owned)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    };
                    let (released, unknown) = (ids("released"), ids("unknown"));
                    for id in &released {
                        lines.push(check(
                            Some(false),
                            format!(
                                "claim: {id} — dead session, will return to Todo on next claim"
                            ),
                        ));
                    }
                    for id in &unknown {
                        lines.push(check(
                            None,
                            format!("claim: {id} — running on another host"),
                        ));
                    }
                    if released.is_empty() && unknown.is_empty() {
                        lines.push(check(Some(true), "claims: none stale".into()));
                    }
                }
                Err(_) => lines.push(check(
                    None,
                    "claims: could not probe (reap --dry-run failed)".into(),
                )),
            }
        }
        Self::display_message(DOCTOR_CUSTOM_TYPE, lines.join("\n"), None).await;
    }

    async fn show(&self, cx: &Ctx, all: bool, lane: Option<&str>) {
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        let Some(slug) = self.ensure_onboarded(cx).await else {
            return;
        };
        let tasks: Vec<Task> = match self.list(cx, &["--all"]).await {
            Ok(t) => t
                .into_iter()
                .filter(|t| lane.is_none_or(|l| t.status == l))
                .collect(),
            Err(e) => {
                notify(&format!("kanboard: show failed — {e}"), "error");
                return;
            }
        };
        let plain = render::show_plain(&slug, &tasks, all, lane);
        let details = json!({"project": slug, "tasks": tasks, "all": all, "lane": lane});
        Self::display_message(SHOW_CUSTOM_TYPE, plain, Some(details)).await;
    }

    async fn kanboard_command(&'static self, args: &str, cx: &Ctx) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let sub = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();
        let rest = args
            .trim()
            .get(parts.first().map_or(0, |p| p.len())..)
            .unwrap_or("")
            .trim()
            .to_owned();
        match sub.as_str() {
            "" => Self::display_message(HELP_CUSTOM_TYPE, HELP.to_owned(), None).await,
            "add" => notify(
                "kanboard: use /unipi:kanboard-add <title> — lines below the title become the description",
                "info",
            ),
            "work" => notify(
                "kanboard: use /unipi:kanboard-autowork start to work every ready task in this session",
                "info",
            ),
            "stop" => notify(
                "kanboard: use /unipi:kanboard-autowork stop to turn autowork offers off",
                "info",
            ),
            "onboard" => {
                self.onboard(cx).await;
            }
            "close" => self.stop_daemon(cx).await,
            "status" => {
                if let Some(why) = self.unavailable(cx).await {
                    notify(&format!("kanboard: {why}"), "warning");
                } else {
                    self.status(cx).await;
                }
            }
            "doctor" => self.doctor(cx).await,
            "show" => {
                let lane = parts
                    .iter()
                    .find(|p| args::SHOW_FILTERS.contains(p))
                    .copied();
                self.show(cx, parts.contains(&"--all"), lane).await;
            }
            "open" => self.open(cx, &rest).await,
            other => notify(
                &format!(
                    "kanboard: unknown subcommand \"{other}\" — /unipi:kanboard lists the commands"
                ),
                "warning",
            ),
        }
    }

    async fn do_command(&self, args: &str, cx: &Ctx) {
        let request = args.trim();
        if request.eq_ignore_ascii_case("off") {
            self.sessions.with(cx.session_id(), |s| s.guard.revoke());
            notify("kanboard: -do budget revoked — reads stay free", "info");
            return;
        }
        if request.is_empty() {
            notify(
                "kanboard: -do needs a request — /unipi:kanboard-do <what the agent may do on the board>",
                "warning",
            );
            return;
        }
        if let Some(why) = self.unavailable(cx).await {
            notify(&format!("kanboard: {why}"), "warning");
            return;
        }
        let Some(slug) = self.ensure_onboarded(cx).await else {
            return;
        };
        Self::reveal_skill();
        let cfg = self.config();
        let budget = self.sessions.with(cx.session_id(), |s| {
            s.guard.open(cfg.do_tasks, cfg.do_writes);
            s.monitor.arm();
            s.tracker.on_prompt(request);
            s.guard.remaining()
        });
        let text = args::do_text(&slug, &self.binary(), request, budget.slots, budget.writes);
        Self::send_harness_user(
            &text,
            "Task budget",
            &format!(
                "slots {} · writes {} · user request included",
                budget.slots, budget.writes
            ),
        )
        .await;
        self.sessions
            .with(cx.session_id(), |s| s.guard.note_sent(timers::now_ms()));
    }

    async fn set_autowork(&self, cx: &Ctx, on: bool) {
        self.sessions.with(cx.session_id(), |s| {
            s.guard.set_autowork(on);
            s.monitor.set_autowork(on);
            if on {
                s.monitor.arm();
            }
        });
        self.refresh_holder(cx).await;
    }

    async fn autowork_command(&self, args: &str, cx: &Ctx) {
        match args.trim().to_lowercase().as_str() {
            "start" => {
                let Some(slug) = self.ensure_onboarded(cx).await else {
                    return;
                };
                if let Some(why) = self.unavailable(cx).await {
                    notify(&format!("kanboard: {why}"), "warning");
                    return;
                }
                Self::reveal_skill();
                self.set_autowork(cx, true).await;
                Self::send_harness_user(
                    &args::autowork_text(&slug, &self.binary()),
                    "Autowork",
                    "no budget limits · user request included",
                )
                .await;
            }
            "stop" => {
                self.set_autowork(cx, false).await;
                notify("kanboard: autowork off", "info");
            }
            _ => notify("kanboard: /unipi:kanboard-autowork start|stop", "warning"),
        }
    }

    async fn cached_tasks(&self, cx: &Ctx) -> Vec<Task> {
        let now = timers::now_ms();
        if let Some(tasks) = self.sessions.with(cx.session_id(), |s| {
            s.cache
                .as_ref()
                .filter(|(at, _)| now - at < 5_000)
                .map(|(_, t)| t.clone())
        }) {
            return tasks;
        }
        let tasks = self.list(cx, &["--all"]).await.unwrap_or_default();
        self.sessions
            .with(cx.session_id(), |s| s.cache = Some((now, tasks.clone())));
        tasks
    }

    // ── panel ───────────────────────────────────────────────────────────

    async fn panel_body(&self, cx: &Ctx) -> Option<(String, Value)> {
        let slug = self.slug(cx).await?;
        let mut tasks = self.list(cx, &["--all"]).await.unwrap_or_default();
        let (selected, live) = self
            .sessions
            .with(cx.session_id(), |s| (s.selected.clone(), s.live));
        if let Some(sel) = &selected
            && let Ok(v) = self.run(cx, &["show", sel], Some(&slug), true).await
            && let Ok(full) = shapes::as_task("show", &v)
            && let Some(slot) = tasks.iter_mut().find(|t| &t.id == sel)
        {
            *slot = full;
        }
        let tag = Self::session_tag(cx);
        Some((
            slug.clone(),
            panel::spec(&slug, &tasks, selected.as_deref(), &tag, live),
        ))
    }

    async fn open_panel(&'static self, cx: &Ctx) {
        let Some((slug, body)) = self.panel_body(cx).await else {
            notify(
                "kanboard: not set up here — /unipi:kanboard onboard",
                "warning",
            );
            return;
        };
        if ui::panel_open(&panel::open_request(&slug, &body)).is_err() {
            return;
        }
        let generation = self.sessions.with(cx.session_id(), |s| {
            s.panel_open = true;
            s.panel_gen += 1;
            s.panel_gen
        });
        if let Some(d) = self.daemon(cx).await {
            let cx = cx.clone();
            wit_spawn(async move { self.follow_sse(&cx, d, &slug, generation).await });
        }
    }

    async fn refresh_panel(&self, cx: &Ctx) {
        if !self.sessions.with(cx.session_id(), |s| s.panel_open) {
            return;
        }
        if let Some((slug, body)) = self.panel_body(cx).await {
            let _ = ui::panel_update(
                panel::PANEL_ID,
                &json!({"spec": body, "title": format!("Kanboard · {slug}")}),
            );
        }
    }

    async fn follow_sse(
        &'static self,
        cx: &Ctx,
        daemon: shapes::DaemonInfo,
        slug: &str,
        generation: u64,
    ) {
        use oino_extension::futures::StreamExt;
        let mut req = http::get(&format!(
            "http://127.0.0.1:{}/events?project={slug}",
            daemon.port
        ));
        if let Some(t) = daemon.token.as_deref() {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        let Ok(resp) = req.send().await else {
            return;
        };
        if resp.status() != 200 {
            return;
        }
        self.sessions.with(cx.session_id(), |s| s.live = true);
        let mut stream = resp.body().into_stream();
        let mut buffer = String::new();
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(b) = stream.next().await {
            if self.sessions.with(cx.session_id(), |s| {
                !s.panel_open || s.panel_gen != generation
            }) {
                return;
            }
            bytes.push(b);
            if b != b'\n' {
                continue;
            }
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            bytes.clear();
            if !panel::drain_sse(&mut buffer).is_empty() {
                self.refresh_panel(cx).await;
            }
        }
        self.sessions.with(cx.session_id(), |s| s.live = false);
    }

    async fn on_ui_action(&'static self, ev: &Event, cx: &Ctx) {
        let panel_id = s(&ev.payload, "panelId");
        if !panel_id.ends_with(panel::PANEL_ID) {
            return;
        }
        let action = s(&ev.payload, "actionId");
        let value = ev.payload.get("value").cloned().unwrap_or(Value::Null);
        match action.as_str() {
            "$close" => self.sessions.with(cx.session_id(), |s| {
                s.panel_open = false;
                s.live = false;
            }),
            panel::ACTION_SELECT => {
                let id = value.as_str().map(ToOwned::to_owned);
                self.sessions.with(cx.session_id(), |s| s.selected = id);
                self.refresh_panel(cx).await;
            }
            panel::ACTION_REFRESH => self.refresh_panel(cx).await,
            panel::ACTION_SHOW => self.show(cx, false, None).await,
            panel::ACTION_ADD => {
                if let Ok(Some(text)) =
                    ui::input("Add to Backlog", Some("title (lines below = description)")).await
                    && !text.trim().is_empty()
                {
                    self.add(cx, &text).await;
                }
                self.refresh_panel(cx).await;
            }
            a if a.starts_with(panel::ACTION_MOVE) => {
                let to = &a[panel::ACTION_MOVE.len()..];
                let selected = self.sessions.with(cx.session_id(), |s| s.selected.clone());
                if let (Some(id), Some(slug)) = (selected, self.slug(cx).await) {
                    self.panel_move(cx, &slug, &id, to).await;
                }
                self.refresh_panel(cx).await;
            }
            _ => {}
        }
    }

    async fn panel_move(&self, cx: &Ctx, slug: &str, id: &str, to: &str) {
        match self.run(cx, &["move", id, to], Some(slug), true).await {
            Ok(_) => notify(&format!("kanboard: {id} → {to}"), "info"),
            Err(e) if panel::needs_comment(&e.message) => {
                let Ok(Some(comment)) =
                    ui::input(&format!("{id} → {to}: comment"), Some(&e.message)).await
                else {
                    return;
                };
                if comment.trim().is_empty() {
                    return;
                }
                match self
                    .run(
                        cx,
                        &["move", id, to, "--comment", comment.trim()],
                        Some(slug),
                        true,
                    )
                    .await
                {
                    Ok(_) => notify(&format!("kanboard: {id} → {to}"), "info"),
                    Err(e) => notify(&format!("kanboard: {e}"), "error"),
                }
            }
            Err(e) => notify(&format!("kanboard: {e}"), "error"),
        }
    }

    // ── run hooks ───────────────────────────────────────────────────────

    async fn on_tool_call(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        self.sessions.with(cx.session_id(), |s| s.tool_calls += 1);
        let tool = s(&ev.payload, "toolName");
        if tool != "bash" && tool != "powershell" {
            return None;
        }
        let input = ev.payload.get("input").cloned().unwrap_or(Value::Null);
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if shell::invocations(&command).is_empty() {
            return None;
        }
        let tag = Self::session_tag(cx);
        let slug = self.slug(cx).await;
        let mut owned = HashMap::new();
        for id in WriteGuard::claim_probes(&command) {
            let mine = match &slug {
                Some(slug) => self
                    .run(cx, &["show", &id], Some(slug), true)
                    .await
                    .ok()
                    .and_then(|v| shapes::as_task("show", &v).ok())
                    .is_some_and(|t| t.claimed_by(&tag)),
                None => false,
            };
            owned.insert(id, mine);
        }
        let cfg = self.config();
        let child = self.is_child(cx);
        let reason = self.sessions.with(cx.session_id(), |s| {
            s.guard.check(&command, child, cfg.turn_add_limit, &owned)
        });
        if let Some(reason) = reason {
            return Some(json!({"block": true, "reason": reason}));
        }
        // The agent's own calls carry the session identity (unipi: process env).
        let mut vars = vec![
            ("OINO_KANBOARD_SESSION", tag),
            ("OINO_KANBOARD_PID", process::self_pid().to_string()),
            ("OINO_KANBOARD_MAX_SESSIONS", cfg.max_sessions.to_string()),
        ];
        if let Some(slug) = slug {
            vars.push(("OINO_KANBOARD_PROJECT", slug));
        }
        let mut input = input;
        input["command"] = json!(format!("{}{command}", env_prefix(&tool, &vars)));
        Some(json!({"input": input}))
    }

    async fn on_tool_result(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        let tool = s(&ev.payload, "toolName");
        let input = ev.payload.get("input").cloned().unwrap_or(Value::Null);
        let is_error = ev.payload.get("isError").and_then(Value::as_bool) == Some(true);
        let child = self.is_child(cx);
        if !child && (tool == "bash" || tool == "powershell") {
            if let Some(badge) = render::badge_from_tool_call(&tool, &input, is_error) {
                let _ = session::append_entry(BADGE_ENTRY, Some(&badge.to_json()));
            }
            if !is_error {
                let invs =
                    shell::invocations(input.get("command").and_then(Value::as_str).unwrap_or(""));
                if invs
                    .iter()
                    .any(|i| matches!(i.sub.as_str(), "start" | "finish" | "move"))
                {
                    if invs.iter().any(|i| i.sub == "start") {
                        self.sessions.with(cx.session_id(), |s| s.monitor.arm());
                    }
                    self.refresh_holder(cx).await;
                }
            }
        }
        let silent = !self.config().reminders || child;
        let candidates = self.sessions.with(cx.session_id(), |s| {
            s.tracker.observe(&tool, &input, is_error, silent)
        })?;
        let tasks = self.list(cx, &[]).await.ok()?;
        let todo = self
            .sessions
            .with(cx.session_id(), |s| s.tracker.remind(&candidates, &tasks))?;
        let cli = self.cli_prefix(cx).await;
        let content = ev
            .payload
            .get("content")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        let details = ev.payload.get("details").cloned().unwrap_or(Value::Null);
        Some(reminders::r1_result(
            &content,
            &details,
            &todo,
            cli.as_deref(),
            &self.next_id("r1"),
        ))
    }

    async fn on_before_agent_start(&self, ev: &Event, cx: &Ctx) {
        let prompt = s(&ev.payload, "prompt");
        self.sessions
            .with(cx.session_id(), |s| s.tracker.on_prompt(&prompt));
        if let Some(ids) = Monitor::prompt_ids(&prompt)
            && let Ok(tasks) = self.list(cx, &[]).await
        {
            let tag = Self::session_tag(cx);
            self.sessions.with(cx.session_id(), |s| {
                s.monitor.on_user_prompt(&ids, &tasks, &tag);
            });
        }
    }

    fn on_agent_end(&self, ev: &Event, cx: &Ctx) {
        let messages = ev
            .payload
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (text, tools) = monitor::last_assistant(&messages);
        self.sessions.with(cx.session_id(), |s| {
            s.guard.on_agent_end(timers::now_ms());
            s.monitor.on_agent_end(&messages);
            s.settle = SettleInfo {
                last_assistant_text: text,
                last_assistant_had_tool_calls: tools,
                tool_calls_this_run: s.tool_calls,
            };
        });
    }

    async fn on_settled(&self, cx: &Ctx) {
        let notices = self
            .sessions
            .with(cx.session_id(), |s| std::mem::take(&mut s.notices));
        for n in notices {
            let _ = session::append_entry(
                NOTICE_ENTRY,
                Some(&json!({"text": n.text, "level": n.level})),
            );
            if cx.has_ui() {
                notify(&n.text, n.level);
            }
        }
        self.refresh_holder(cx).await;
    }

    async fn propose(&self, cx: &Ctx) -> Option<Value> {
        if self.is_child(cx) {
            return None;
        }
        let (armed, wants_ready, info) = self.sessions.with(cx.session_id(), |s| {
            (s.monitor.armed(), s.monitor.wants_ready(), s.settle.clone())
        });
        if !armed {
            return None;
        }
        let owner =
            unipi_kit::bus::get::<LhState>().map_or_else(OwnerStatus::default, |st| OwnerStatus {
                owner_status: st.owner.map(|o| o.status),
                last_stop: st.last_stop.map(|l| (l.kind, l.at)),
            });
        let tasks = self.list(cx, &[]).await;
        let ready = if wants_ready {
            Some(self.list(cx, &["--ready"]).await)
        } else {
            None
        };
        let cli = self.cli_prefix(cx).await;
        let tag = Self::session_tag(cx);
        let out = self.sessions.with(cx.session_id(), |s| {
            let out = s.monitor.propose(
                &info,
                &tag,
                &owner,
                tasks.as_deref().ok(),
                ready,
                cli.as_deref(),
            );
            s.notices.extend(out.notices.iter().cloned());
            if out.autowork_off {
                s.guard.set_autowork(false);
            }
            out
        });
        if out.autowork_off {
            self.refresh_holder(cx).await;
        }
        out.proposal.map(|p| p.to_json())
    }

    // ── lifecycle ───────────────────────────────────────────────────────

    async fn on_session_start(&self, cx: &Ctx) {
        if self.own_child.get() {
            return;
        }
        let attached = self.attach(cx).await;
        unipi_kit::bus::emit(&ModuleReady {
            name: "@pi-unipi/kanboard".into(),
            version: VERSION.into(),
            commands: vec!["kanboard".into()],
            tools: Vec::new(),
            load_time_ms: None,
        });
        if attached.is_err() {
            return;
        }
        if !cx.is_child() {
            self.sync_runtime(cx).await;
        }
        let slug = self.slug(cx).await;
        if !cx.is_child()
            && let Some(slug) = &slug
        {
            let _ = self.run(cx, &["reap"], Some(slug), true).await;
        }
        self.refresh_holder(cx).await;
        if !cx.is_child() {
            let tag = Self::session_tag(cx);
            let stale: Vec<String> = self
                .list(cx, &[])
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|t| {
                    t.status == "todo"
                        && t.activity.last().is_some_and(|last| {
                            last.text.starts_with(&format!("session lost: {tag}"))
                                || last
                                    .text
                                    .contains(&format!("released: session ended mid-task ({tag})"))
                        })
                })
                .map(|t| t.id)
                .collect();
            if !stale.is_empty() {
                notify(
                    &format!(
                        "kanboard: {} was released when the last run ended — ask me to re-start it",
                        stale.join(", ")
                    ),
                    "info",
                );
            }
        }
        let cfg = self.config();
        if (cfg.archive_after_days > 0 || cfg.retention_days > 0)
            && let Some(slug) = &slug
        {
            let after = cfg.archive_after_days.to_string();
            let retention = cfg.retention_days.to_string();
            let _ = self
                .run(
                    cx,
                    &[
                        "archive-sweep",
                        "--after-days",
                        &after,
                        "--retention-days",
                        &retention,
                    ],
                    Some(slug),
                    true,
                )
                .await;
        }
    }

    async fn on_session_shutdown(&self, cx: &Ctx) {
        if self.is_child(cx) {
            return;
        }
        let tag = Self::session_tag(cx);
        let claims = self.own_claims(cx).await;
        let Some(slug) = self.slug(cx).await else {
            return;
        };
        if claims.is_empty() {
            return;
        }
        let comment = format!("released: session ended mid-task ({tag})");
        for task in &claims {
            let _ = self
                .run(
                    cx,
                    &[
                        "--actor",
                        "system",
                        "--project",
                        &slug,
                        "release",
                        &task.id,
                        "--to",
                        "todo",
                        "--comment",
                        &comment,
                    ],
                    Some(&slug),
                    true,
                )
                .await;
        }
        let autowork = self
            .sessions
            .with(cx.session_id(), |s| s.guard.remaining().autowork);
        let cli = self.cli_prefix(cx).await;
        unipi_kit::bus::emit(&KanboardStatus {
            claims: Vec::new(),
            autowork,
            cli,
        });
        self.sessions.remove(cx.session_id());
    }

    async fn move_handle(&self, cx: &Ctx, req: &MoveRequest) -> MoveReport {
        let mut log = Vec::new();
        let Ok(v) = self.run(cx, &["project", "list"], None, true).await else {
            log.push("kanboard: binary unavailable — nothing to rebind".into());
            return MoveReport { log };
        };
        let projects = v.as_array().cloned().unwrap_or_default();
        let Some(project) = projects.iter().find(|p| s(p, "root") == req.old_root) else {
            return MoveReport { log };
        };
        let slug = s(project, "slug");
        log.push(format!(
            "kanboard: project {slug}: rebind root {} → {} via oino-kanboard project rebind",
            req.old_root, req.new_root
        ));
        if req.dry_run {
            return MoveReport { log };
        }
        match self
            .run(
                cx,
                &["project", "rebind", &slug, "--root", &req.new_root],
                None,
                true,
            )
            .await
        {
            Ok(_) => {
                self.slugs.borrow_mut().insert(req.new_root.clone(), slug);
                log.push("kanboard: rebind ok".into());
            }
            Err(e) => log.push(format!("kanboard: rebind failed: {e}")),
        }
        MoveReport { log }
    }

    fn render(req: &Value) -> Option<Value> {
        let data = req.get("data")?;
        match req.get("key").and_then(Value::as_str)? {
            SHOW_CUSTOM_TYPE => render::show_node(data.get("details")?),
            BADGE_ENTRY => render::badge_node(data.get("data")?),
            NOTICE_ENTRY => render::notice_node(data.get("data")?),
            PROGRESS_ENTRY => render::progress_node(data.get("data")?),
            _ => None,
        }
    }
}

/// Drops display-only messages from the LLM context.
#[must_use]
pub fn context_filter(messages: &[Value]) -> Option<Vec<Value>> {
    const HIDDEN: &[&str] = &[
        HELP_CUSTOM_TYPE,
        DOCTOR_CUSTOM_TYPE,
        SHOW_CUSTOM_TYPE,
        NOTICE_ENTRY,
    ];
    let kept: Vec<Value> = messages
        .iter()
        .filter(|m| !HIDDEN.contains(&m.get("customType").and_then(Value::as_str).unwrap_or("")))
        .cloned()
        .collect();
    (kept.len() != messages.len()).then_some(kept)
}

fn register_commands(cx: &mut Init<Kanboard>) {
    cx.command(
        Command::<Kanboard>::new("unipi:kanboard")
            .description("Kanboard — the board: open, onboard, status, doctor (bare = help)")
            .argument_hint("[open|close|onboard|status|show [--all]|doctor]")
            .run(|ext, args, cx| async move {
                ext.kanboard_command(&args, &cx.cx).await;
                Ok(())
            })
            .complete(|_ext, prefix, _cx| async move { args::kanboard_completions(&prefix) }),
    );
    cx.command(
        Command::<Kanboard>::new("unipi:kanboard-add")
            .description("Add a board task — [-p 1-5] [--after ID] [--status backlog|todo] <title>; lines below are the description (paste file paths to attach)")
            .argument_hint("[-p 1-5] [--after ID] [--status backlog|todo] <title>")
            .run(|ext, args, cx| async move {
                ext.add(&cx.cx, &args).await;
                Ok(())
            })
            .complete(|ext, prefix, cx| async move {
                match args::add_completions(&prefix) {
                    AddCompletion::Items(items) => items,
                    AddCompletion::Tasks(p, before) => args::task_completions(&ext.cached_tasks(&cx).await, &p, &before, true, false),
                }
            }),
    );
    cx.command(
        Command::<Kanboard>::new("unipi:kanboard-do")
            .description("Grant the agent kanboard task slots and a write budget (/unipi:kanboard-do <request>; off revokes)")
            .argument_hint("<request>|off")
            .run(|ext, args, cx| async move {
                ext.do_command(&args, &cx.cx).await;
                Ok(())
            })
            .complete(|ext, prefix, cx| async move {
                let Some((token, before, title_search)) = args::do_completion_target(&prefix) else {
                    return Vec::<CompletionItem>::new();
                };
                args::task_completions(&ext.cached_tasks(&cx).await, &token, &before, title_search, true)
            }),
    );
    cx.command(
        Command::<Kanboard>::new("unipi:kanboard-autowork")
            .description("Work ready board tasks one by one — start | stop (stop finishes the current task first)")
            .argument_hint("start|stop")
            .run(|ext, args, cx| async move {
                ext.autowork_command(&args, &cx.cx).await;
                Ok(())
            })
            .complete(|_ext, prefix, _cx| async move { args::autowork_completions(&prefix) }),
    );
    cx.command(
        Command::<Kanboard>::new(ACTION_OPEN)
            .description("Kanboard: start the daemon and open the board")
            .hidden()
            .run(|ext, _args, cx| async move {
                ext.open(&cx.cx, "").await;
                Ok(())
            }),
    );
    cx.command(
        Command::<Kanboard>::new(ACTION_STOP_DAEMON)
            .description("Kanboard: stop the board daemon")
            .hidden()
            .run(|ext, _args, cx| async move {
                ext.stop_daemon(&cx.cx).await;
                Ok(())
            }),
    );
    cx.command(
        Command::<Kanboard>::new(ACTION_ROTATE_TOKEN)
            .description("Kanboard: rotate the access token")
            .hidden()
            .run(|ext, _args, cx| async move {
                ext.rotate_token(&cx.cx).await;
                Ok(())
            }),
    );
}

fn register_services(cx: &mut Init<Kanboard>) {
    cx.service(
        point::KANBOARD_CAPTURE,
        |ext, payload, _caller, cx| async move { Ok(ext.capture(&cx, &payload).await) },
    );
    cx.service(fan::EVIDENCE_CONTRIBUTE, |ext, _payload, _caller, cx| async move {
        let blocking = if ext.is_child(&cx) {
            Vec::new()
        } else {
            ext.own_claims(&cx)
                .await
                .into_iter()
                .map(|t| format!("{} is still In Progress (claimed by this session) — finish it or move it to blocked", t.id))
                .collect()
        };
        Ok(serde_json::to_value(EvidenceContribution { blocking, notes: Vec::new() })?)
    });
    cx.service(
        fan::COMPACTION_CONTEXT_PROVIDE,
        |_ext, _payload, _caller, _cx| async move {
            let status = unipi_kit::bus::get::<KanboardStatus>();
            let text = status.and_then(|st| args::claims_reminder(&st.claims, st.cli.as_deref()));
            Ok(serde_json::to_value(CompactionContext {
                id: "kanboard".into(),
                text,
            })?)
        },
    );
    cx.service(fan::MOVE_HANDLE, |ext, payload, _caller, cx| async move {
        let req: MoveRequest = serde_json::from_value(payload)?;
        Ok(serde_json::to_value(ext.move_handle(&cx, &req).await)?)
    });
}

fn register_events(cx: &mut Init<Kanboard>) {
    cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
        ext.on_session_start(&cx).await;
        Ok(())
    });
    cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
        ext.on_session_shutdown(&cx).await;
        Ok(())
    });
    cx.on(EventKind::BeforeAgentStart, |ext, ev, cx| async move {
        ext.on_before_agent_start(&ev, &cx).await;
        Ok(())
    });
    cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
        ext.sessions.with(cx.session_id(), |s| {
            s.tracker.on_turn_start();
            s.monitor.on_agent_start(timers::now_ms());
            s.tool_calls = 0;
        });
        Ok(())
    });
    cx.on(EventKind::AgentEnd, |ext, ev, cx| async move {
        ext.on_agent_end(&ev, &cx);
        Ok(())
    });
    cx.on(EventKind::AgentSettled, |ext, _ev, cx| async move {
        ext.on_settled(&cx).await;
        Ok(())
    });
    cx.on(EventKind::Context, |_ext, ev, _cx| async move {
        let messages = ev
            .payload
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(context_filter(&messages).map(|kept| json!({"messages": kept})))
    });
    cx.on(EventKind::ToolCall, |ext, ev, cx| async move {
        Ok(ext.on_tool_call(&ev, &cx).await)
    });
    cx.on(EventKind::ToolResult, |ext, ev, cx| async move {
        Ok(ext.on_tool_result(&ev, &cx).await)
    });
    cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
        ext.on_ui_action(&ev, &cx).await;
        Ok(())
    });
    cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
        if let Some(d) = unipi_kit::bus::Delivery::from_event(&ev)
            && let Some(change) = d.decode::<LhOwnerChanged>()
            && change.event == "resumed"
        {
            ext.sessions.with(cx.session_id(), |s| s.monitor.arm());
        }
        Ok(())
    });
    unipi_kit::bus::subscribe(&[topics::LONG_HORIZON_OWNER_CHANGED]);
}

impl Extension for Kanboard {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        self.own_child.set(
            host::env("OINO_KANBOARD_CHILD")
                .ok()
                .flatten()
                .is_some_and(|v| !v.is_empty()),
        );
        register_commands(cx);
        register_services(cx);
        register_events(cx);
        for (target, key) in [
            (RendererTarget::CustomMessage, SHOW_CUSTOM_TYPE),
            (RendererTarget::CustomEntry, BADGE_ENTRY),
            (RendererTarget::CustomEntry, NOTICE_ENTRY),
            (RendererTarget::CustomEntry, PROGRESS_ENTRY),
        ] {
            cx.renderer(target, key, |_ext, req, _cx| Kanboard::render(&req));
        }
        cx.nudge_provider(|ext, cx| async move { ext.propose(&cx).await });
        cx.on_nudge_delivered(|ext, id, cx| {
            ext.sessions
                .with(cx.session_id(), |s| s.monitor.delivered(&id));
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Kanboard);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_prefix_quotes_for_both_shells() {
        let vars = [("A", "x'y".to_owned()), ("B", "2".to_owned())];
        assert_eq!(env_prefix("bash", &vars), "export A='x'\\''y' B='2'\n");
        assert_eq!(
            env_prefix("powershell", &vars),
            "$env:A='x''y'; $env:B='2'; "
        );
        // The rewritten command still parses as the same invocation.
        let cmd = format!("{}oino-kanboard start A-1", env_prefix("bash", &vars));
        assert_eq!(shell::invocations(&cmd)[0].sub, "start");
    }

    #[test]
    fn context_filter_drops_display_only_types() {
        let msgs = [
            json!({"role": "custom", "customType": HELP_CUSTOM_TYPE}),
            json!({"role": "user"}),
        ];
        assert_eq!(
            context_filter(&msgs).unwrap(),
            vec![json!({"role": "user"})]
        );
        assert!(context_filter(&[json!({"role": "user"})]).is_none());
    }

    #[test]
    fn render_dispatch() {
        let req = json!({"key": BADGE_ENTRY, "data": {"data": {"kind": "finished", "id": "A-1"}}});
        assert!(Kanboard::render(&req).is_some());
        assert!(Kanboard::render(&json!({"key": "x", "data": {}})).is_none());
    }
}
