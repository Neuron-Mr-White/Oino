//! `unipi.background-tasks` — long shell commands that wake the agent when they finish
//! (unipi `packages/background-tasks`, unipi-port §3.9).
//!
//! The host owns processes (`tasks` interface, extension-api §5.5): spawn, output file, cap,
//! timeout, kill, the last 30 finished tasks. This component is the unipi surface on top:
//!
//! | unipi | here |
//! |---|---|
//! | `bg_run`, `bg_status`, `bg_logs`, `bg_kill` (schemas and texts) | tools over `tasks.*` |
//! | `/bg [--agent] [--name "N"] <command>`, `/bg-tasks [id]` | commands |
//! | `Ctrl+Alt+C` clear finished notices | shortcut `unipi.bg.clear` |
//! | `<background-task-notification>` (followUp, wakes when `triggerOnCompletion`) | service `tasks.notification` renders the text the host delivers |
//! | launch/completion cards | renderers for `bg_run`, `bg_logs`, `bg_kill` and the notification message |
//! | work-tray "Background tasks" tab | `work-tray.tab` / `work-tray.action` services ([`pane`]) |
//! | arbiter wait source `bg: <name> +N more` | `wait-source` export ([`text::pending_wake_reason`]) |
//! | shared registry (`Symbol.for`) | sticky topic `unipi:bg:tasks` `{tasks: [BgTaskSnapshot]}` |
//! | `pi.events` request/response/terminal API v1 | bus topics ([`api`]) |
//! | settings `background-tasks` (master toggle) | namespace ([`settings`]); `enabled: false` registers nothing else |

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

pub mod api;
pub mod cards;
pub mod pane;
pub mod settings;
pub mod snapshot;
pub mod text;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use oino_extension::wit::registry::RendererTarget;
use unipi_kit::bus::Delivery;
use unipi_kit::services::{OpenTray, fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::topics;
use unipi_kit::tray::{Action, ActionReply, TabRequest, TrayTab};

use crate::pane::{Armed, LogView, PaneAction, PaneState};
use crate::settings::{Config, NAMESPACE};
use crate::snapshot::{HostTask, Local, Snapshot, Status};

/// Timer refreshing the open tray while tasks run.
const TICK: &str = "bg-tray-tick";
/// unipi `stopWaitMs` analogue: how long kill waits for the host to report the end.
const STOP_WAIT_MS: i64 = 5_000;
/// Shutdown reason recorded on tasks stopped by `session_shutdown`.
const SHUTDOWN_REASON: &str = "Killed during Oino session shutdown/reload";

#[derive(Default)]
struct SessionState {
    local: HashMap<String, Local>,
    seen: HashSet<String>,
    dismissed: HashSet<String>,
    pane: PaneState,
    tray_open: bool,
    ticking: bool,
}

/// The extension.
#[derive(Default)]
pub struct Bg {
    settings: Store,
    sessions: SessionMap<SessionState>,
    seen_requests: RefCell<HashSet<String>>,
    shutting_down: Cell<bool>,
}

fn obj_str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

impl Bg {
    fn config(&self) -> Config {
        Config::from_values(&self.settings.get(NAMESPACE))
    }

    /// Host tasks of this session, minus dismissed ones, finished capped at `maxFinishedTasks`.
    fn host_tasks(&self, sid: &str) -> Vec<HostTask> {
        let all: Vec<HostTask> = tasks::list()
            .unwrap_or_default()
            .iter()
            .filter_map(HostTask::decode)
            .collect();
        let dismissed = self.sessions.with(sid, |s| s.dismissed.clone());
        let mut list: Vec<HostTask> = all
            .into_iter()
            .filter(|t| !dismissed.contains(&t.id))
            .collect();
        let max = self.config().max_finished_tasks;
        let mut finished: Vec<(i64, String)> = list
            .iter()
            .filter(|t| t.state != "running")
            .map(|t| (t.finished_at.unwrap_or(t.started_at), t.id.clone()))
            .collect();
        if finished.len() > max {
            finished.sort();
            let drop: HashSet<String> = finished[..finished.len() - max]
                .iter()
                .map(|(_, id)| id.clone())
                .collect();
            list.retain(|t| !drop.contains(&t.id));
        }
        list
    }

    fn output(&self, id: &str) -> String {
        let cap = u32::try_from(self.config().max_output_bytes).unwrap_or(u32::MAX);
        tasks::logs(id, cap, true).unwrap_or_default()
    }

    fn snapshot(&self, sid: &str, host: &HostTask, cwd: &str) -> Snapshot {
        let local = self
            .sessions
            .with(sid, |s| s.local.get(&host.id).cloned().unwrap_or_default());
        let out = self.output(&host.id);
        Snapshot::build(host, &local, Some(&out), cwd)
    }

    fn snapshots(&self, cx: &Ctx) -> Vec<Snapshot> {
        let cwd = cx.cwd().to_string_lossy().into_owned();
        self.host_tasks(cx.session_id())
            .iter()
            .map(|t| self.snapshot(cx.session_id(), t, &cwd))
            .collect()
    }

    fn resolve(&self, cx: &Ctx, id: &str) -> Result<HostTask> {
        let list = self.host_tasks(cx.session_id());
        snapshot::resolve(&list, id)
            .cloned()
            .map_err(Error::Message)
    }

    /// Publishes `unipi:bg:tasks` and the tray change signal.
    fn publish(&self, cx: &Ctx) {
        let snaps = self.snapshots(cx);
        unipi_kit::bus::emit_raw(topics::BG_TASKS, &json!({"tasks": snaps}));
        unipi_kit::bus::emit_raw(topics::WORK_TRAY_CHANGED, &json!({"tab": pane::TAB}));
        let (open, ticking) = self
            .sessions
            .with(cx.session_id(), |s| (s.tray_open, s.ticking));
        let running = snaps.iter().any(|t| t.status == Status::Running);
        if open && running && !ticking {
            timers::set(TICK, Duration::from_secs(1), true);
            self.sessions.with(cx.session_id(), |s| s.ticking = true);
        }
    }

    // ── starting ────────────────────────────────────────────────────────────

    /// unipi `registry.startTask`.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors unipi StartTaskOptions one to one"
    )]
    fn start(
        &self,
        cx: &Ctx,
        command: &str,
        name: Option<&str>,
        description: Option<&str>,
        is_agent: bool,
        timeout_seconds: Option<u64>,
        notify: bool,
        trigger: bool,
    ) -> Result<Snapshot> {
        let command = command.trim();
        if command.is_empty() {
            return Err(Error::msg("Background command is empty"));
        }
        if self.shutting_down.get() {
            return Err(Error::msg(
                "Cannot start a background task while Oino is shutting down",
            ));
        }
        let display = text::task_display_name(name, description, Some(command), None);
        let mut spec = json!({
            "name": display,
            "command": command,
            "isAgent": is_agent,
            "notifyOnCompletion": notify,
            "triggerOnCompletion": trigger,
            "maxOutputBytes": self.config().max_output_bytes,
        });
        if let Some(d) = description {
            spec["description"] = json!(d);
        }
        if let Some(t) = timeout_seconds {
            spec["timeoutSeconds"] = json!(t);
        }
        let id = tasks::start(&spec)
            .map_err(|e| Error::msg(format!("Failed to start background task: {e}")))?;
        let cwd = cx.cwd().to_string_lossy().into_owned();
        self.sessions.with(cx.session_id(), |s| {
            s.local.insert(
                id.clone(),
                Local {
                    cwd: cwd.clone(),
                    ..Local::default()
                },
            );
        });
        let host = tasks::get(&id)?
            .as_ref()
            .and_then(HostTask::decode)
            .ok_or_else(|| Error::msg(format!("Unknown background task ID: {id}")))?;
        self.publish(cx);
        Ok(self.snapshot(cx.session_id(), &host, &cwd))
    }

    // ── stopping ────────────────────────────────────────────────────────────

    /// unipi `stopTask` / `killTask`: kills and waits for the host to report the end.
    async fn stop(&self, cx: &Ctx, host: &HostTask, reason: Option<&str>) -> Result<Snapshot> {
        if host.state != "running" {
            let status = Status::from_host(&host.state);
            return Err(Error::msg(format!(
                "Task {} is {}, not running",
                host.id,
                status.as_str()
            )));
        }
        self.sessions.with(cx.session_id(), |s| {
            let l = s.local.entry(host.id.clone()).or_default();
            l.stopped = true;
            l.error = reason.map(ToOwned::to_owned);
        });
        tasks::kill(&host.id)?;
        let deadline = timers::now_ms() + STOP_WAIT_MS;
        loop {
            let now = tasks::get(&host.id)?.as_ref().and_then(HostTask::decode);
            match now {
                Some(t) if t.state != "running" => {
                    let cwd = cx.cwd().to_string_lossy().into_owned();
                    return Ok(self.snapshot(cx.session_id(), &t, &cwd));
                }
                None => {
                    return Err(Error::msg(format!(
                        "Unknown background task ID: {}",
                        host.id
                    )));
                }
                Some(_) if timers::now_ms() >= deadline => {
                    return Err(Error::msg(format!(
                        "Task {} did not exit within {} after cancellation",
                        host.id,
                        text::format_duration(STOP_WAIT_MS)
                    )));
                }
                Some(_) => timers::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    async fn stop_all(&self, cx: &Ctx, reason: Option<&str>) -> (usize, Vec<String>) {
        let running: Vec<HostTask> = self
            .host_tasks(cx.session_id())
            .into_iter()
            .filter(|t| t.state == "running")
            .collect();
        let mut stopped = 0;
        let mut failures = Vec::new();
        for t in &running {
            match self.stop(cx, t, reason).await {
                Ok(_) => stopped += 1,
                Err(e) => failures.push(format!("{} ({}): {e}", t.name, t.id)),
            }
        }
        (stopped, failures)
    }

    // ── tools ───────────────────────────────────────────────────────────────

    fn bg_run(&self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let Some(is_agent) = args.get("isAgent").and_then(Value::as_bool) else {
            return Err(Error::msg(
                "bg_run requires isAgent boolean. Set true only for LLM/agent tasks; set false for scripts, tests, servers, sleeps.",
            ));
        };
        let cfg = self.config();
        let notify = args
            .get("notifyOnCompletion")
            .and_then(Value::as_bool)
            .unwrap_or(cfg.notify_on_completion);
        let trigger = args
            .get("triggerOnCompletion")
            .and_then(Value::as_bool)
            .unwrap_or(cfg.trigger_on_completion);
        let timeout = args
            .get("timeoutSeconds")
            .and_then(Value::as_f64)
            .filter(|t| *t > 0.0)
            .map(|t| {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "positive seconds"
                )]
                let s = t.ceil() as u64;
                s
            });
        let task = self.start(
            &cx.cx,
            obj_str(args, "command").unwrap_or_default(),
            obj_str(args, "name"),
            obj_str(args, "description"),
            is_agent,
            timeout,
            notify,
            trigger,
        )?;
        let (_, guidance) = text::completion_delivery_guidance(
            task.notify_on_completion,
            task.trigger_on_completion,
        );
        let pid = task
            .pid
            .map_or_else(|| "unknown".to_owned(), |p| p.to_string());
        Ok(ToolResult::text(format!(
            "Started background task {} ({})\nStatus: {}\nPID: {pid}\nOutput: {}\n{guidance}",
            task.display_name(),
            task.id,
            task.status.as_str(),
            task.output_path
        ))
        .with_details(json!({"task": task})))
    }

    fn bg_status(&self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let snaps = match obj_str(args, "taskId").filter(|s| !s.is_empty()) {
            Some(id) => {
                let host = self.resolve(&cx.cx, id)?;
                let cwd = cx.cx.cwd().to_string_lossy().into_owned();
                vec![self.snapshot(cx.session_id(), &host, &cwd)]
            }
            None => self.snapshots(&cx.cx),
        };
        Ok(
            ToolResult::text(text::format_snapshot_list(&snaps, timers::now_ms()))
                .with_details(json!({"tasks": snaps})),
        )
    }

    /// unipi `getTaskLogs` → `(text, details)`.
    fn logs(&self, cx: &Ctx, id: &str, max_bytes: u32, tail: bool) -> Result<(String, Value)> {
        let host = self.resolve(cx, id)?;
        let full = self.output(&host.id);
        let bytes = full.as_bytes();
        let take = (max_bytes as usize).min(bytes.len());
        let slice = if tail {
            &bytes[bytes.len() - take..]
        } else {
            &bytes[..take]
        };
        let content = String::from_utf8_lossy(slice).into_owned();
        let (text, truncated) =
            text::logs_text(&content, bytes.len() as u64, tail, &host.output_path);
        let cwd = cx.cwd().to_string_lossy().into_owned();
        let task = self.snapshot(cx.session_id(), &host, &cwd);
        let details = json!({
            "task": task, "path": host.output_path, "bytesRead": take, "truncated": truncated, "tail": tail,
        });
        Ok((text, details))
    }

    fn bg_logs(&self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let id = obj_str(args, "taskId").unwrap_or_default();
        let max = text::normalize_max_bytes(
            args.get("maxBytes").and_then(Value::as_f64),
            text::DEFAULT_LOG_BYTES,
        );
        let tail = args.get("tail").and_then(Value::as_bool).unwrap_or(true);
        let (text, details) = self.logs(&cx.cx, id, max, tail)?;
        Ok(ToolResult::text(text).with_details(details))
    }

    async fn bg_kill(&self, args: &Value, cx: &ToolCtx) -> Result<ToolResult> {
        let host = self.resolve(&cx.cx, obj_str(args, "taskId").unwrap_or_default())?;
        let task = self.stop(&cx.cx, &host, None).await?;
        let message = format!(
            "Killed background task {} ({}). Output: {}",
            task.display_name(),
            task.id,
            task.output_path
        );
        Ok(ToolResult::text(message.clone())
            .with_details(json!({"task": task, "message": message})))
    }

    // ── commands ────────────────────────────────────────────────────────────

    fn bg_command(&self, args: &str, cx: &CommandCtx) {
        let started = text::parse_bg_command_args(args)
            .map_err(Error::Message)
            .and_then(|p| {
                self.start(
                    &cx.cx,
                    &p.command,
                    p.name.as_deref(),
                    None,
                    p.is_agent,
                    None,
                    true,
                    false,
                )
            });
        let _ = match started {
            Ok(t) => ui::notify(
                &format!(
                    "Started {} ({})\nOutput: {}\nCommand: {}",
                    t.display_name(),
                    t.id,
                    t.output_path,
                    t.command
                ),
                "info",
            ),
            Err(e) => ui::notify(&format!("Background task failed to start: {e}"), "error"),
        };
    }

    async fn open_task_manager(&self, cx: &Ctx, initial: Option<String>) {
        if !cx.has_ui() {
            let _ = ui::notify(
                "Background task manager requires an interactive UI. Use the bg_status/bg_logs tools in non-interactive mode.",
                "error",
            );
            return;
        }
        if let Some(id) = &initial {
            self.sessions.with(cx.session_id(), |s| {
                s.pane.view = Some(id.clone());
                s.pane.full_log = false;
            });
        }
        let request = OpenTray {
            tab: Some(pane::TAB.into()),
            initial_id: initial,
        };
        let _ = services::call(
            Some(unipi_kit::suite::pkg::CORE.id),
            point::CORE_OPEN_TRAY,
            &serde_json::to_value(request).unwrap_or_default(),
            None,
        )
        .await;
    }

    fn clear_finished_notices(&self, cx: &Ctx) -> usize {
        let finished: Vec<String> = self
            .host_tasks(cx.session_id())
            .into_iter()
            .filter(|t| t.state != "running")
            .map(|t| t.id)
            .collect();
        let n = self.sessions.with(cx.session_id(), |s| {
            let before = s.seen.len();
            s.seen.extend(finished);
            s.seen.len() - before
        });
        self.publish(cx);
        n
    }

    // ── work tray ───────────────────────────────────────────────────────────

    fn tab(&self, cx: &Ctx, req: &TabRequest) -> TrayTab {
        let snaps = self.snapshots(cx);
        if let Some(id) = &req.initial_id
            && snaps.iter().any(|t| &t.id == id)
        {
            self.sessions
                .with(cx.session_id(), |s| s.pane.view = Some(id.clone()));
        }
        self.sessions
            .with(cx.session_id(), |s| s.tray_open = req.open);
        let body = req.open.then(|| self.body(cx, &snaps));
        TrayTab {
            id: pane::TAB.into(),
            label: "Background tasks".into(),
            short_label: Some("Bg tasks".into()),
            order: 0,
            counts: pane::counts(&snaps),
            visible: true,
            body,
            owner: None,
        }
    }

    fn body(&self, cx: &Ctx, snaps: &[Snapshot]) -> Value {
        let state = self.sessions.with(cx.session_id(), |s| s.pane.clone());
        let now = timers::now_ms();
        let Some(view) = state
            .view
            .as_ref()
            .and_then(|id| snaps.iter().find(|t| &t.id == id))
        else {
            self.sessions.with(cx.session_id(), |s| s.pane.view = None);
            return pane::list_body(snaps, &state, now);
        };
        let cap = if state.full_log {
            pane::FULL_LOG_BYTES
        } else {
            pane::TAIL_BYTES
        };
        let log = tasks::logs(&view.id, cap, true).map_or_else(
            |e| LogView {
                error: Some(format!("Output read failed: {e}")),
                ..LogView::default()
            },
            |content| LogView::from_read(&content, view.bytes_written),
        );
        pane::detail_body(view, Some(&log), &state, now)
    }

    fn flash(&self, cx: &Ctx, message: impl Into<String>) {
        let m = message.into();
        self.sessions
            .with(cx.session_id(), |s| s.pane.flash = Some(m));
    }

    #[allow(clippy::too_many_lines, reason = "one arm per unipi pane key")]
    async fn tray_action(&self, cx: &Ctx, action: Action) -> ActionReply {
        let armed = self.sessions.with(cx.session_id(), |s| {
            s.pane.flash = None;
            s.pane.armed.take()
        });
        let Some(act) = PaneAction::parse(&action.action_id, action.value.as_ref()) else {
            return ActionReply::default();
        };
        let find = |id: &str| self.resolve(cx, id).ok();
        match act {
            PaneAction::Open(id) => self.sessions.with(cx.session_id(), |s| {
                s.pane.view = Some(id);
                s.pane.full_log = false;
            }),
            PaneAction::Back => self.sessions.with(cx.session_id(), |s| s.pane.view = None),
            PaneAction::FullLog(_) => self.sessions.with(cx.session_id(), |s| {
                s.pane.full_log = !s.pane.full_log;
            }),
            PaneAction::Stop(id) => {
                if let Some(t) = find(&id) {
                    let name = t.name.clone();
                    if t.state == "running" {
                        match self.stop(cx, &t, None).await {
                            Ok(_) => self.flash(cx, format!("Stopped {name}.")),
                            Err(e) => self.flash(cx, format!("Stopping {name} failed: {e}")),
                        }
                    } else {
                        self.flash(cx, format!("{name} already ended."));
                    }
                }
            }
            PaneAction::Kill(id) => {
                if let Some(t) = find(&id) {
                    let name = t.name.clone();
                    if t.state != "running" {
                        self.flash(cx, format!("{name} already ended."));
                    } else if armed != Some(Armed::Kill(id.clone())) {
                        self.sessions
                            .with(cx.session_id(), |s| s.pane.armed = Some(Armed::Kill(id)));
                        self.flash(cx, format!("Press x again to kill {name} now (SIGKILL)."));
                    } else {
                        match self.stop(cx, &t, None).await {
                            Ok(_) => self.flash(cx, format!("Killed {name}.")),
                            Err(e) => self.flash(cx, format!("Killing {name} failed: {e}")),
                        }
                    }
                }
            }
            PaneAction::StopAll => {
                let running = self
                    .host_tasks(cx.session_id())
                    .iter()
                    .filter(|t| t.state == "running")
                    .count();
                if running == 0 {
                    self.flash(cx, "No running background tasks.");
                } else if armed != Some(Armed::StopAll) {
                    self.sessions
                        .with(cx.session_id(), |s| s.pane.armed = Some(Armed::StopAll));
                    self.flash(
                        cx,
                        format!(
                            "Press a again to stop all {}.",
                            text::tray_plural(running, "running task")
                        ),
                    );
                } else {
                    let (stopped, failures) = self.stop_all(cx, None).await;
                    self.flash(
                        cx,
                        if failures.is_empty() {
                            format!("Stopped {}.", text::tray_plural(stopped, "task"))
                        } else {
                            format!(
                                "Stopped {stopped}; {} failed: {}",
                                failures.len(),
                                failures.join("; ")
                            )
                        },
                    );
                }
            }
            PaneAction::Dismiss(id) => {
                if let Some(t) = find(&id) {
                    if t.state == "running" {
                        self.flash(cx, format!("{} is still running — s stop first.", t.name));
                    } else {
                        self.sessions.with(cx.session_id(), |s| {
                            s.dismissed.insert(id.clone());
                            s.seen.insert(id.clone());
                            s.pane.view = None;
                        });
                        self.flash(cx, format!("Dismissed {}.", t.name));
                    }
                }
            }
            PaneAction::DismissAll => {
                let finished: Vec<String> = self
                    .host_tasks(cx.session_id())
                    .into_iter()
                    .filter(|t| t.state != "running")
                    .map(|t| t.id)
                    .collect();
                let n = finished.len();
                self.sessions.with(cx.session_id(), |s| {
                    s.seen.extend(finished.iter().cloned());
                    s.dismissed.extend(finished);
                });
                self.flash(
                    cx,
                    if n > 0 {
                        format!("Dismissed {}.", text::tray_plural(n, "finished task"))
                    } else {
                        "No finished tasks to dismiss.".to_owned()
                    },
                );
            }
            PaneAction::Rerun(id) => {
                if let Some(t) = find(&id) {
                    match self.start(
                        cx,
                        &t.command,
                        Some(&t.name),
                        t.description.as_deref(),
                        t.is_agent,
                        t.timeout_seconds,
                        true,
                        false,
                    ) {
                        Ok(r) => {
                            self.sessions.with(cx.session_id(), |s| {
                                if s.pane.view.is_some() {
                                    s.pane.view = Some(r.id.clone());
                                }
                            });
                            self.flash(cx, format!("Reran as {}.", r.id));
                        }
                        Err(e) => self.flash(cx, format!("Rerunning {} failed: {e}", t.name)),
                    }
                }
            }
            PaneAction::Path(id) => {
                if let Some(t) = find(&id) {
                    let _ = ui::notify(
                        &format!("Output path for {} ({}):\n{}", t.name, t.id, t.output_path),
                        "info",
                    );
                    self.flash(cx, format!("Log: {}", t.output_path));
                }
            }
        }
        self.publish(cx);
        ActionReply {
            close: false,
            refresh: true,
        }
    }

    // ── events ──────────────────────────────────────────────────────────────

    fn on_task(&self, ev: &Event, cx: &Ctx) {
        let id = obj_str(&ev.payload, "id").unwrap_or_default().to_owned();
        let state = obj_str(&ev.payload, "state").unwrap_or("running");
        if state != "running"
            && let Ok(Some(info)) = tasks::get(&id)
            && let Some(host) = HostTask::decode(&info)
        {
            let cwd = cx.cwd().to_string_lossy().into_owned();
            let snap = self.snapshot(cx.session_id(), &host, &cwd);
            let _ = bus::publish(
                api::TERMINAL_CHANNEL,
                &api::terminal(&serde_json::to_value(&snap).unwrap_or_default()),
                false,
            );
        }
        self.publish(cx);
    }

    async fn on_bus(&self, ev: &Event, cx: &Ctx) {
        let Some(d) = Delivery::from_event(ev) else {
            return;
        };
        if d.topic != api::REQUEST_CHANNEL {
            return;
        }
        let response = match api::parse_request(&d.data) {
            Err(r) => api::error_response(&r.request_id, &r.operation, &r.error),
            Ok(req) => {
                let fresh = self
                    .seen_requests
                    .borrow_mut()
                    .insert(req.request_id.clone());
                if fresh {
                    match self.execute(cx, &req).await {
                        Ok(result) => api::success_response(&req, &result),
                        Err(e) => api::error_response(
                            &req.request_id,
                            req.operation.name(),
                            &e.to_string(),
                        ),
                    }
                } else {
                    api::error_response(
                        &req.request_id,
                        req.operation.name(),
                        &format!("duplicate request_id {}", req.request_id),
                    )
                }
            }
        };
        let _ = bus::publish(api::RESPONSE_CHANNEL, &response, false);
    }

    async fn execute(&self, cx: &Ctx, req: &api::Request) -> Result<Value> {
        if self.shutting_down.get() {
            return Err(Error::msg(
                "unipi-background-tasks EventBus service is shutting down",
            ));
        }
        let cwd = cx.cwd().to_string_lossy().into_owned();
        Ok(match &req.operation {
            api::Operation::Capabilities => api::capabilities(),
            api::Operation::Run {
                name,
                command,
                is_agent,
                timeout_seconds,
                notify_on_completion,
                trigger_on_completion,
            } => serde_json::to_value(self.start(
                cx,
                command,
                Some(name),
                None,
                *is_agent,
                *timeout_seconds,
                *notify_on_completion,
                *trigger_on_completion,
            )?)?,
            api::Operation::Status { task_id } => {
                let snaps = match task_id {
                    Some(id) => vec![self.snapshot(cx.session_id(), &self.resolve(cx, id)?, &cwd)],
                    None => self.snapshots(cx),
                };
                json!({"tasks": snaps})
            }
            api::Operation::Logs {
                task_id,
                max_bytes,
                tail,
            } => {
                #[allow(clippy::cast_precision_loss, reason = "byte counts")]
                let max =
                    text::normalize_max_bytes(max_bytes.map(|b| b as f64), text::DEFAULT_LOG_BYTES);
                let (text, mut details) = self.logs(cx, task_id, max, tail.unwrap_or(true))?;
                details["text"] = json!(text);
                details
            }
            api::Operation::Kill { task_id } => {
                let host = self.resolve(cx, task_id)?;
                let snap = self.stop(cx, &host, None).await?;
                let message = format!(
                    "Killed background task {} ({}). Output: {}",
                    snap.display_name(),
                    snap.id,
                    snap.output_path
                );
                json!({"task": snap, "message": message})
            }
        })
    }

    async fn on_shutdown_event(&self, cx: &Ctx) {
        self.shutting_down.set(true);
        timers::cancel(TICK);
        let (_, failures) = self.stop_all(cx, Some(SHUTDOWN_REASON)).await;
        if !failures.is_empty() && cx.has_ui() {
            let _ = ui::notify(
                &format!("Background task cleanup failed:\n{}", failures.join("\n")),
                "error",
            );
        }
    }

    fn on_tick(&self, cx: &Ctx) {
        let (open, any_running) = (
            self.sessions.with(cx.session_id(), |s| s.tray_open),
            self.host_tasks(cx.session_id())
                .iter()
                .any(|t| t.state == "running"),
        );
        if open && any_running {
            unipi_kit::bus::emit_raw(topics::WORK_TRAY_CHANGED, &json!({"tab": pane::TAB}));
        } else {
            timers::cancel(TICK);
            self.sessions.with(cx.session_id(), |s| s.ticking = false);
        }
    }

    // ── rendering ───────────────────────────────────────────────────────────

    fn render(&self, req: &Value, cx: &Ctx) -> Option<Value> {
        let expanded = req
            .get("expanded")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let data = req.get("data")?;
        let key = obj_str(req, "key")?;
        if key == NOTIFICATION_TYPE {
            let id = data.pointer("/details/taskId").and_then(Value::as_str)?;
            let host = tasks::get(id)
                .ok()
                .flatten()
                .as_ref()
                .and_then(HostTask::decode);
            let snap = host.map(|h| {
                let cwd = cx.cwd().to_string_lossy().into_owned();
                self.snapshot(cx.session_id(), &h, &cwd)
            });
            return Some(cards::completion_card(snap.as_ref(), expanded));
        }
        let details = data.pointer("/result/details")?;
        let task: Snapshot = serde_json::from_value(details.get("task")?.clone()).ok()?;
        match key {
            "bg_run" => Some(cards::launch_card(&task, expanded)),
            "bg_kill" => Some(cards::kill_card(&task)),
            "bg_logs" => {
                let output = expanded
                    .then(|| {
                        data.pointer("/result/content/0/text")
                            .and_then(Value::as_str)
                    })
                    .flatten();
                Some(cards::logs_card(
                    &task,
                    details.get("tail").and_then(Value::as_bool).unwrap_or(true),
                    details
                        .get("bytesRead")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    details
                        .get("truncated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    obj_str(details, "path").unwrap_or_default(),
                    output,
                ))
            }
            _ => None,
        }
    }
}

/// Custom message type of completion notifications.
pub const NOTIFICATION_TYPE: &str = "background-task-notification";

fn tool_schemas() -> [(&'static str, Value); 4] {
    [
        (
            "bg_run",
            json!({"type": "object", "properties": {
                "name": {"type": "string", "description": "Short human-readable task name shown in the bg footer dock. Required; use 2-6 words, not the raw command."},
                "command": {"type": "string", "description": "Shell command to start in the background"},
                "isAgent": {"type": "boolean", "description": "True ONLY when the command launches an LLM/agent process (enables telemetry wrapping). False for scripts, tests, servers, sleeps."},
                "description": {"type": "string", "description": "Longer human-readable description"},
                "timeoutSeconds": {"type": "number", "description": "Kill the task after this many seconds (optional)"},
                "notifyOnCompletion": {"type": "boolean", "description": "Deliver a durable terminal notification when the task finishes. Default true for finite tasks. Set false or pair with triggerOnCompletion:false for persistent servers/daemons/watchers."},
                "triggerOnCompletion": {"type": "boolean", "description": "Start a follow-up agent turn on terminal notification. Default true for finite tasks (tests, builds, exports). MUST set false for persistent background services/servers (e.g. dev servers, watchers) so the harness and subagents do not wait indefinitely for the server to finish. Requires notifyOnCompletion."},
            }, "required": ["name", "command", "isAgent"]}),
        ),
        (
            "bg_status",
            json!({"type": "object", "properties": {
                "taskId": {"type": "string", "description": "Task ID or unambiguous prefix. Omit to list all tasks."},
            }}),
        ),
        (
            "bg_logs",
            json!({"type": "object", "properties": {
                "taskId": {"type": "string", "description": "Task ID or unambiguous prefix"},
                "maxBytes": {"type": "number", "description": format!("Maximum bytes of output to return (1-{}). Default {}.", text::MAX_LOG_BYTES, text::DEFAULT_LOG_BYTES)},
                "tail": {"type": "boolean", "description": "Read from the end (tail, default true) or the beginning (head)"},
            }, "required": ["taskId"]}),
        ),
        (
            "bg_kill",
            json!({"type": "object", "properties": {
                "taskId": {"type": "string", "description": "Task ID or unambiguous prefix of a running task"},
            }, "required": ["taskId"]}),
        ),
    ]
}

/// Tool schemas by name (exact unipi `TypeBox` shapes).
#[must_use]
pub fn tool_schema(name: &str) -> Value {
    tool_schemas()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map_or(Value::Null, |(_, s)| s)
}

fn register_tools(cx: &mut Init<Bg>) {
    cx.tool(
        Tool::<Bg>::new("bg_run")
            .label("Background Run")
            .description("Start a named long-running shell command in the background and return immediately with a task ID and output path. By default, terminal state is delivered automatically as <background-task-notification> and starts a follow-up agent turn; do not sleep or poll merely to wait. Output is written under the OS temp root and model-visible logs are bounded.")
            .prompt_snippet("Start a named long-running shell command; default terminal notification wakes a follow-up turn, so yield instead of polling")
            .guideline("Use bg_run instead of bash for commands expected to run in the background (test suites, builds, dev servers, watchers).")
            .guideline("For FINITE tasks (tests, builds, exports, scripts): leave triggerOnCompletion:true and notifyOnCompletion:true (defaults) to automatically wake the agent when finished.")
            .guideline("For PERSISTENT servers/daemons/watchers (e.g. 'npm run dev', vite, live dev servers, background listeners): MUST set triggerOnCompletion:false (and optionally notifyOnCompletion:false) so the agent and sidekick do not wait for the server to exit.")
            .guideline("Set isAgent:true only when the background task launches an LLM/agent process; false for scripts, tests, dev servers, sleeps.")
            .guideline("Always set name to a concise 2-6 word human-readable label; do not use the raw command as the name.")
            .guideline("bg_run returns immediately. For finite tasks with default wake-up, continue only independent useful work that does not merely wait for the task; otherwise end the current turn. Do not call sleep, bg_status, or bg_logs merely to wait; the terminal notification will wake you.")
            .guideline("A running result is not an instruction to poll again.")
            .guideline("Treat <background-task-notification> as durable terminal truth. Do not call bg_status to reconfirm it.")
            .parameters(&tool_schema("bg_run"))
            .kind(ToolKind::Execute)
            .render_hints(&json!({"title": "bg_run {name}"}))
            .run(|ext, args: Value, cx| async move { ext.bg_run(&args, &cx) }),
    );
    cx.tool(
        Tool::<Bg>::new("bg_status")
            .label("Background Status")
            .description("Inspect one background task or list all running/recent background tasks. Point-in-time inspection tool, not a waiting primitive.")
            .prompt_snippet("Inspect point-in-time status for one or all background tasks; never poll it as a wait loop")
            .guideline("Use bg_status for deliberate point-in-time inspection, not as a waiting primitive.")
            .guideline("A running result is not an instruction to poll again.")
            .guideline("Use bg_status when the user explicitly requests an update, automatic completion handling was disabled, or there is concrete evidence a task is hung.")
            .parameters(&tool_schema("bg_status"))
            .kind(ToolKind::Read)
            .render_hints(&json!({"title": "bg_status {taskId}"}))
            .run(|ext, args: Value, cx| async move { ext.bg_status(&args, &cx) }),
    );
    cx.tool(
        Tool::<Bg>::new("bg_logs")
            .label("Background Logs")
            .description("Read bounded output from a background task for deliberate inspection; not a waiting primitive. Output is capped for model safety and points to the full output file when truncated.")
            .prompt_snippet("Read bounded task output when needed; never tail it repeatedly as a wait loop")
            .guideline("Use bg_logs with a modest maxBytes value only when task output is needed.")
            .guideline("Do not repeatedly call bg_logs to wait for completion while an automatic terminal notification is pending.")
            .parameters(&tool_schema("bg_logs"))
            .kind(ToolKind::Read)
            .render_hints(&json!({"title": "bg_logs {taskId}"}))
            .run(|ext, args: Value, cx| async move { ext.bg_logs(&args, &cx) }),
    );
    cx.tool(
        Tool::<Bg>::new("bg_kill")
            .label("Background Kill")
            .description("Stop a running background task by ID. Fails loudly if the task is unknown or already finished.")
            .prompt_snippet("Stop a running background task by ID")
            .guideline("Use bg_kill when the user asks to stop a background task or when a bg_run command is no longer needed.")
            .parameters(&tool_schema("bg_kill"))
            .kind(ToolKind::Execute)
            .render_hints(&json!({"title": "bg_kill {taskId}"}))
            .run(|ext, args: Value, cx| async move { ext.bg_kill(&args, &cx).await }),
    );
}

fn register_ui(cx: &mut Init<Bg>) {
    cx.command(
        Command::<Bg>::new("bg")
            .description("Start a shell command as a tracked background task: /bg [--agent] [--name \"Task name\"] <command>")
            .argument_hint("[--agent] [--name \"N\"] <command>")
            .run(|ext, args, cx| async move {
                ext.bg_command(&args, &cx);
                Ok(())
            }),
    );
    cx.command(
        Command::<Bg>::new("bg-tasks")
            .description("Open the work tray on the Background tasks tab")
            .argument_hint("[task-id]")
            .run(|ext, args, cx| async move {
                let id = args.trim();
                ext.open_task_manager(&cx.cx, (!id.is_empty()).then(|| id.to_owned()))
                    .await;
                Ok(())
            }),
    );
    cx.shortcut(
        Shortcut::<Bg>::new("unipi.bg.clear", &["ctrl+alt+c"])
            .description("Clear finished background task footer notices")
            .run(|ext, cx| async move {
                ext.clear_finished_notices(&cx.cx);
                Ok(())
            }),
    );
    for (target, key) in [
        (RendererTarget::CustomMessage, NOTIFICATION_TYPE),
        (RendererTarget::Tool, "bg_run"),
        (RendererTarget::Tool, "bg_logs"),
        (RendererTarget::Tool, "bg_kill"),
    ] {
        cx.renderer(target, key, |ext, req, cx| ext.render(&req, &cx));
    }
    cx.service(fan::WORK_TRAY_TAB, |ext, payload, _caller, cx| async move {
        let req: TabRequest = serde_json::from_value(payload).unwrap_or_default();
        Ok(json!([ext.tab(&cx, &req)]))
    });
    cx.service(
        point::WORK_TRAY_ACTION,
        |ext, payload, _caller, cx| async move {
            let action: Action = serde_json::from_value(payload)?;
            Ok(serde_json::to_value(ext.tray_action(&cx, action).await)?)
        },
    );
}

impl Extension for Bg {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = settings::namespace();
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&ns.to_json());
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        // Master toggle: when off, register nothing else (unipi `if (!config.enabled) return`).
        if !self.config().enabled {
            return Ok(());
        }
        register_tools(cx);
        register_ui(cx);
        cx.service(
            point::TASKS_NOTIFICATION,
            |ext, payload, _caller, cx| async move {
                let host =
                    HostTask::decode(&payload).ok_or_else(|| Error::msg("TaskInfo expected"))?;
                ext.sessions.with(cx.session_id(), |s| {
                    s.local.entry(host.id.clone()).or_default().notified = true;
                });
                let cwd = cx.cwd().to_string_lossy().into_owned();
                let snap = ext.snapshot(cx.session_id(), &host, &cwd);
                Ok(json!(text::notification_text(&snap)))
            },
        );
        cx.wait_source(|ext, cx| async move {
            if cx.is_child() {
                return None;
            }
            text::pending_wake_reason(&ext.snapshots(&cx))
        });
        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.shutting_down.set(false);
            ext.publish(&cx);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.on_shutdown_event(&cx).await;
            Ok(())
        });
        cx.on(EventKind::OinoTask, |ext, ev, cx| async move {
            ext.on_task(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            if obj_str(&ev.payload, "timerId") == Some(TICK) {
                ext.on_tick(&cx);
            }
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            ext.on_bus(&ev, &cx).await;
            Ok(())
        });
        bus::subscribe(api::REQUEST_CHANNEL);
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Bg);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_match_unipi() {
        assert_eq!(
            tool_schema("bg_run")["required"],
            json!(["name", "command", "isAgent"])
        );
        assert_eq!(
            tool_schema("bg_logs")["properties"]["maxBytes"]["description"],
            "Maximum bytes of output to return (1-30720). Default 30720."
        );
        assert_eq!(tool_schema("bg_status").get("required"), None);
        assert_eq!(tool_schema("bg_kill")["required"], json!(["taskId"]));
        assert_eq!(tool_schema("nope"), Value::Null);
    }
}
