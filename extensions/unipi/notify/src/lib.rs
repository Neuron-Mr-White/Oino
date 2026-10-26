//! `unipi.notify` — notifications for agent events and the `notify_user` tool (unipi
//! `packages/notify`, unipi-port §3.8).
//!
//! | unipi | here |
//! |---|---|
//! | platforms native (node-notifier) / Gotify / Telegram / ntfy | `ui.os-notify` / HTTP ([`platforms`]) |
//! | `notify_user` tool (schema, result text, fire-and-forget) | same; dispatch deferred to a 0 ms timer |
//! | event matrix (`events.<e>.{enabled,platforms}`), priorities 2/5/8 and 2/3/5 | same ([`config`], [`messages`]) |
//! | `agent_settled` "finished" after 1.5 s of quiet, one "All done" after pending work | same; pending = running `unipi:bg:tasks` with `triggerOnCompletion` or host run state `waiting`; watch over the work topics |
//! | `input_needed` on `ui_prompt_start` while running, de-dup 2 s after ask/permission alerts | same |
//! | renotify every `intervalMs` (≤ `maxRepeats`, "(still waiting)", high) | repeating `oino_timer`; stops on input, `agent_start`, last prompt closed, shutdown |
//! | silence after input (`onTerminalInput`) | editor intents (`oino_editor`) and `input` events |
//! | recap via the model registry | `models.complete` ([`recap`]) |
//! | `/unipi:notify-event <event> on\|off`, runners set-gotify/set-ntfy/set-telegram/test | same names as commands (runners hidden); setup wizards as dialogs |
//! | `NOTIFICATION_SENT` | same topic |
//! | app push encryption, UnifiedPush fan-out, `unipi://` deep links, herdr | dropped (unipi-port §2.5) |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

mod config;
mod messages;
mod platforms;
mod recap;

use std::cell::RefCell;
use std::time::Duration;

use oino_extension::command::item;
use oino_extension::futures::future::join_all;
use oino_extension::kv::Scope;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::bus::Delivery;
use unipi_kit::settings::{Store, WriteScope, deep_merge, set_field};
use unipi_kit::topics::{self, NotificationSent};

use crate::config::{EVENTS, NAMESPACE, NotifyConfig, PLATFORMS, SECRET_KEYS};
use crate::messages::{PRODUCT, is_blocking, title_for};
use crate::platforms::PlatformResult;

/// Kv key of the local settings overlay (fallback when a settings write is refused; imported
/// into the global settings file once).
const OVERLAY_KEY: &str = "settings-overlay";
/// unipi `FINISH_GRACE_MS`.
const FINISH_GRACE_MS: u64 = 1500;
/// unipi `ALL_DONE_GRACE_MS`.
const ALL_DONE_GRACE_MS: u64 = 2500;
/// unipi `PROMPT_DEDUP_MS`.
const PROMPT_DEDUP_MS: i64 = 2000;
/// Telegram chat-id polling limit (unipi `TIMEOUT_MS`).
const TELEGRAM_POLL_MS: i64 = 5 * 60 * 1000;
/// Foreign permission prompt channel (`@gotgenes/pi-permission-system`).
const PERMISSION_UI_PROMPT: &str = "permissions:ui_prompt";
/// Topics that change pending work (unipi `subscribeWorkChanges`).
const WORK_TOPICS: &[&str] = &[
    topics::BG_TASKS,
    topics::SUBAGENTS_LIST,
    topics::FUSION_STATUS,
    topics::WORK_TRAY_CHANGED,
];

/// A queued notification (sent from a timer so hooks never wait on the network).
#[derive(Clone, Debug)]
struct Job {
    title: String,
    message: String,
    platforms: Vec<String>,
    event: String,
    priority: Option<String>,
}

#[derive(Clone, Debug)]
struct Renotify {
    job: Job,
    fired: u64,
    max: u64,
}

#[derive(Default)]
struct SessionState {
    agent_running: bool,
    open_prompts: u32,
    last_blocking_alert_at: i64,
    last_input_at: i64,
    renotify: Option<Renotify>,
    pending_done: bool,
    all_done_platforms: Vec<String>,
    all_done_timer: bool,
    finish_payload: Option<Value>,
    queue: Vec<Job>,
}

/// The extension.
#[derive(Default)]
pub struct Notify {
    settings: Store,
    sessions: SessionMap<SessionState>,
    overlay: RefCell<Option<Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Priority {
    Low,
    Normal,
    High,
}

impl Priority {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum PlatformName {
    Native,
    Gotify,
    Telegram,
    Ntfy,
}

impl PlatformName {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Gotify => "gotify",
            Self::Telegram => "telegram",
            Self::Ntfy => "ntfy",
        }
    }
}

/// `notify_user` parameters (unipi `NotifyUserSchema`).
#[derive(Deserialize)]
struct NotifyUserArgs {
    /// Notification message body
    message: String,
    /// Notification title (default: Oino Notification)
    #[serde(default)]
    title: Option<String>,
    /// Priority level
    #[serde(default)]
    priority: Option<Priority>,
    /// Override platforms for this notification
    #[serde(default)]
    platforms: Option<Vec<PlatformName>>,
}

/// unipi `NotifyUserSchema` (`TypeBox`) as JSON Schema.
fn notify_user_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "message": {"type": "string", "description": "Notification message body"},
            "title": {"type": "string", "description": "Notification title (default: Oino Notification)"},
            "priority": {"type": "string", "enum": ["low", "normal", "high"], "description": "Priority level"},
            "platforms": {
                "type": "array",
                "items": {"type": "string", "enum": ["native", "gotify", "telegram", "ntfy"]},
                "description": "Override platforms for this notification",
            },
        },
        "required": ["message"],
    })
}

/// Leaf values of an object as dot-path keys (`{"a": {"b": 1}}` → `[("a.b", 1)]`).
fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(map) if !map.is_empty() || prefix.is_empty() => {
            for (k, child) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, child, out);
            }
        }
        _ if prefix.is_empty() => {}
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

/// Removes a dot-path key; `true` when something was removed.
fn remove_field(v: &mut Value, key: &str) -> bool {
    match key.split_once('.') {
        None => v.as_object_mut().is_some_and(|o| o.remove(key).is_some()),
        Some((head, rest)) => v
            .get_mut(head)
            .is_some_and(|child| remove_field(child, rest)),
    }
}

fn timer_id(kind: &str, sid: &str) -> String {
    format!("notify.{kind}:{sid}")
}

fn parse_timer(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("notify.")?.split_once(':')
}

/// unipi `hasPendingWakeTask`: a running background task that will wake the agent.
fn pending_wake_task(bg: Option<&Value>) -> bool {
    bg.and_then(|v| v.get("tasks"))
        .and_then(Value::as_array)
        .is_some_and(|tasks| {
            tasks.iter().any(|t| {
                t.get("status").and_then(Value::as_str) == Some("running")
                    && t.get("triggerOnCompletion").and_then(Value::as_bool) == Some(true)
            })
        })
}

fn has_pending_work() -> bool {
    let wake = pending_wake_task(unipi_kit::bus::get_raw(topics::BG_TASKS).as_ref());
    let waiting = control::run_state().ok().as_ref().and_then(Value::as_str) == Some("waiting");
    wake || waiting
}

fn time_of_day() -> String {
    let iso = unipi_kit::fmt::iso_from_ms(timers::now_ms());
    iso.get(11..19).unwrap_or(&iso).to_owned() + " UTC"
}

impl Notify {
    // ── configuration ───────────────────────────────────────────────────────

    fn overlay(&self) -> Value {
        if let Some(v) = self.overlay.borrow().as_ref() {
            return v.clone();
        }
        let v = kv::get_json::<Value>(Scope::Global, OVERLAY_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({}));
        *self.overlay.borrow_mut() = Some(v.clone());
        v
    }

    /// Persists a wizard / `/unipi:notify-event` choice in the global settings file (unipi
    /// `setSettings`); the local overlay holds it only when the host refuses the write.
    fn write_setting(&self, key: &str, value: Value) {
        if self
            .settings
            .write(NAMESPACE, key, &value, WriteScope::Global)
            .is_ok()
        {
            self.forget_overlay(&[key.to_owned()]);
            return;
        }
        let v = set_field(&self.overlay(), key, value);
        let _ = kv::set_json(Scope::Global, OVERLAY_KEY, &v);
        *self.overlay.borrow_mut() = Some(v);
    }

    /// One-time import of the overlay into the global settings file (leaf keys; written keys
    /// leave the overlay). Returns the number of keys imported.
    fn import_overlay(&self) -> usize {
        let overlay = self.overlay();
        let mut leaves = Vec::new();
        flatten("", &overlay, &mut leaves);
        let mut imported = Vec::new();
        for (key, value) in leaves {
            if self
                .settings
                .write(NAMESPACE, &key, &value, WriteScope::Global)
                .is_ok()
            {
                imported.push(key);
            }
        }
        self.forget_overlay(&imported);
        if self
            .overlay()
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
        {
            kv::delete(Scope::Global, OVERLAY_KEY);
        }
        imported.len()
    }

    /// Drops overlay keys the host now carries (a real settings write supersedes them).
    fn forget_overlay(&self, keys: &[String]) {
        let mut v = self.overlay();
        let mut changed = false;
        for k in keys {
            changed |= remove_field(&mut v, k);
        }
        if changed {
            let _ = kv::set_json(Scope::Global, OVERLAY_KEY, &v);
            *self.overlay.borrow_mut() = Some(v);
        }
    }

    fn config(&self) -> NotifyConfig {
        let values = config::expand_dotted(&self.settings.get(NAMESPACE).0);
        let merged = deep_merge(&values, &self.overlay());
        let mut cfg = config::from_values(&merged);
        for key in SECRET_KEYS {
            let slot = match key {
                "gotify.appToken" => &mut cfg.gotify.app_token,
                "telegram.botToken" => &mut cfg.telegram.bot_token,
                "telegram.chatId" => &mut cfg.telegram.chat_id,
                _ => &mut cfg.ntfy.token,
            };
            if slot.is_none() {
                *slot = secrets::get(&format!("{NAMESPACE}/{key}"))
                    .ok()
                    .flatten()
                    .or_else(|| secrets::get(key).ok().flatten())
                    .filter(|s| !s.is_empty());
            }
        }
        cfg
    }

    // ── dispatch ────────────────────────────────────────────────────────────

    /// unipi `dispatchNotification`.
    async fn dispatch(&self, sid: &str, job: &Job) -> Vec<PlatformResult> {
        let cfg = self.config();
        let enabled = cfg.resolve_platforms(&job.platforms);
        let last_input = self.sessions.with(sid, |s| s.last_input_at);
        let (send, silenced) = messages::filter_after_input(
            &enabled,
            &cfg.silence_after_input,
            last_input,
            timers::now_ms(),
            &job.event,
        );
        let mut results = join_all(send.iter().map(|p| {
            platforms::send(
                p,
                &job.title,
                &job.message,
                &cfg,
                job.priority.as_deref(),
                Some(job.event.as_str()),
            )
        }))
        .await;
        results.extend(silenced.into_iter().map(|platform| PlatformResult {
            platform,
            success: true,
            suppressed: true,
            ..PlatformResult::default()
        }));
        let unsuppressed: Vec<&PlatformResult> = results.iter().filter(|r| !r.suppressed).collect();
        let success = !results.is_empty() && unsuppressed.iter().all(|r| r.success);
        let suppressed: Vec<String> = results
            .iter()
            .filter(|r| r.suppressed)
            .map(|r| r.platform.clone())
            .collect();
        unipi_kit::bus::emit(&NotificationSent {
            event_type: job.event.clone(),
            platforms: enabled,
            success,
            suppressed_platforms: (!suppressed.is_empty()).then_some(suppressed),
            timestamp: unipi_kit::fmt::iso_from_ms(timers::now_ms()),
        });
        results
    }

    /// Queues `job` and sends it from a 0 ms timer (never blocks the calling hook).
    fn enqueue(&self, sid: &str, job: Job) {
        self.sessions.with(sid, |s| s.queue.push(job));
        timers::set(&timer_id("queue", sid), Duration::ZERO, false);
    }

    async fn drain(&self, sid: &str) {
        let jobs = self.sessions.with(sid, |s| std::mem::take(&mut s.queue));
        for job in jobs {
            let _ = self.dispatch(sid, &job).await;
        }
    }

    // ── renotify ────────────────────────────────────────────────────────────

    fn disarm_renotify(&self, sid: &str) {
        if self.sessions.with(sid, |s| s.renotify.take()).is_some() {
            timers::cancel(&timer_id("renotify", sid));
        }
    }

    fn arm_renotify(&self, sid: &str, job: Job, cfg: &NotifyConfig) {
        self.disarm_renotify(sid);
        let r = &cfg.renotify;
        if !r.enabled || r.max_repeats == 0 {
            return;
        }
        let max = r.max_repeats;
        self.sessions
            .with(sid, |s| s.renotify = Some(Renotify { job, fired: 0, max }));
        timers::set(
            &timer_id("renotify", sid),
            Duration::from_millis(r.interval_ms),
            true,
        );
    }

    async fn renotify_tick(&self, sid: &str) {
        let next = self.sessions.with(sid, |s| {
            let r = s.renotify.as_mut()?;
            r.fired += 1;
            Some((r.job.clone(), r.fired >= r.max))
        });
        let Some((job, last)) = next else {
            timers::cancel(&timer_id("renotify", sid));
            return;
        };
        if last {
            self.disarm_renotify(sid);
        }
        let reminder = Job {
            title: format!("{} (still waiting)", job.title),
            priority: Some("high".into()),
            ..job
        };
        let _ = self.dispatch(sid, &reminder).await;
    }

    /// unipi `notifyBlocking`: high priority + reminder loop + de-dup stamp.
    fn notify_blocking(&self, cx: &Ctx, event: &str, message: String, cfg: &NotifyConfig) {
        let sid = cx.session_id();
        let platforms = cfg
            .event(event)
            .map(|e| e.platforms.clone())
            .unwrap_or_default();
        if event != "input_needed" {
            self.sessions
                .with(sid, |s| s.last_blocking_alert_at = timers::now_ms());
        }
        let job = Job {
            title: title_for(event),
            message,
            platforms,
            event: event.to_owned(),
            priority: Some("high".into()),
        };
        self.enqueue(sid, job.clone());
        self.arm_renotify(sid, job, cfg);
    }

    /// A built-in (non-agent) event fired.
    fn on_builtin(&self, cx: &Ctx, event: &str, payload: &Value) {
        let cfg = self.config();
        let Some(ec) = cfg.event(event).filter(|e| e.enabled).cloned() else {
            return;
        };
        let message = messages::event_message(event, payload);
        if is_blocking(event) {
            self.notify_blocking(cx, event, message, &cfg);
            return;
        }
        self.enqueue(
            cx.session_id(),
            Job {
                title: title_for(event),
                message,
                platforms: ec.platforms,
                event: event.to_owned(),
                priority: messages::default_priority(event).map(ToOwned::to_owned),
            },
        );
    }

    fn note_input(&self, sid: &str) {
        self.sessions
            .with(sid, |s| s.last_input_at = timers::now_ms());
        self.disarm_renotify(sid);
    }

    // ── run finished / all done ─────────────────────────────────────────────

    /// The event key owning "run finished" (`agent_settled` wins over `agent_end`).
    fn finish_key(cfg: &NotifyConfig) -> Option<&'static str> {
        if cfg.event("agent_settled").is_some_and(|e| e.enabled) {
            Some("agent_settled")
        } else if cfg.event("agent_end").is_some_and(|e| e.enabled) {
            Some("agent_end")
        } else {
            None
        }
    }

    fn cancel_finish(&self, sid: &str) {
        if self
            .sessions
            .with(sid, |s| s.finish_payload.take())
            .is_some()
        {
            timers::cancel(&timer_id("finish", sid));
        }
    }

    fn disarm_all_done(&self, sid: &str) {
        let had_timer = self.sessions.with(sid, |s| {
            s.pending_done = false;
            std::mem::take(&mut s.all_done_timer)
        });
        if had_timer {
            timers::cancel(&timer_id("alldone", sid));
        }
    }

    /// unipi `registerAgentNotification` handler, run after the finish grace.
    async fn send_finished(&self, sid: &str, payload: &Value) {
        let cfg = self.config();
        let Some(key) = Self::finish_key(&cfg) else {
            return;
        };
        let platforms = cfg
            .event(key)
            .map(|e| e.platforms.clone())
            .unwrap_or_default();
        if has_pending_work() {
            self.sessions.with(sid, |s| {
                if !s.pending_done {
                    s.pending_done = true;
                    s.all_done_platforms.clone_from(&platforms);
                }
            });
            return;
        }
        let name = session::session_name();
        let mut message = messages::lifecycle_message(key, name.as_deref());
        if cfg.recap.enabled {
            let last = messages::last_assistant_in_messages(payload).or_else(|| {
                session::branch(None)
                    .ok()
                    .and_then(|b| messages::last_assistant_in_entries(&b))
            });
            if let Some(text) = last {
                let recap =
                    recap::summarize(&cfg.recap.model, &text, cfg.recap.disable_thinking).await;
                message = match name.as_deref().filter(|n| !n.is_empty()) {
                    Some(n) => format!("{n}: {recap}"),
                    None => recap,
                };
            }
        }
        let job = Job {
            title: title_for(key),
            message,
            platforms,
            event: key.to_owned(),
            priority: Some("low".into()),
        };
        let _ = self.dispatch(sid, &job).await;
    }

    /// Work topic changed: start the "All done" grace once nothing is pending.
    fn on_work_change(&self, sid: &str) {
        let (armed, timer) = self
            .sessions
            .with(sid, |s| (s.pending_done, s.all_done_timer));
        if !armed || timer || has_pending_work() {
            return;
        }
        self.sessions.with(sid, |s| s.all_done_timer = true);
        timers::set(
            &timer_id("alldone", sid),
            Duration::from_millis(ALL_DONE_GRACE_MS),
            false,
        );
    }

    async fn all_done_tick(&self, sid: &str) {
        let (armed, running, platforms) = self.sessions.with(sid, |s| {
            s.all_done_timer = false;
            (
                s.pending_done,
                s.agent_running,
                s.all_done_platforms.clone(),
            )
        });
        if !armed || running || has_pending_work() {
            return;
        }
        self.sessions.with(sid, |s| s.pending_done = false);
        let job = Job {
            title: format!("{PRODUCT} — All Done"),
            message: messages::all_done_message(session::session_name().as_deref()),
            platforms,
            event: "agent_settled".into(),
            priority: Some("low".into()),
        };
        let _ = self.dispatch(sid, &job).await;
    }

    // ── commands ────────────────────────────────────────────────────────────

    fn notify_event_command(&self, args: &str) {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() != 2 || !matches!(parts[1], "on" | "off") {
            let _ = ui::notify("Usage: /unipi:notify-event <event> <on|off>", "warning");
            return;
        }
        let (event, value) = (parts[0], parts[1]);
        if !EVENTS.contains(&event) {
            let _ = ui::notify(
                &format!(
                    "Unknown event \"{event}\". Known events: {}",
                    EVENTS.join(", ")
                ),
                "error",
            );
            return;
        }
        self.write_setting(&format!("events.{event}.enabled"), json!(value == "on"));
        let _ = ui::notify(&format!("notify: {event} is now {value}."), "info");
    }

    /// unipi `notifyTestHandler`.
    async fn test_command(&self) {
        let cfg = self.config();
        let title = format!("{PRODUCT} — Test Notification");
        let message = format!("Test notification sent at {}", time_of_day());
        let mut lines = Vec::new();
        for p in PLATFORMS {
            let ready = match p {
                "native" => cfg.native.enabled,
                "gotify" => {
                    cfg.gotify.enabled
                        && cfg.gotify.server_url.is_some()
                        && cfg.gotify.app_token.is_some()
                }
                "telegram" => {
                    cfg.telegram.enabled
                        && cfg.telegram.bot_token.is_some()
                        && cfg.telegram.chat_id.is_some()
                }
                _ => cfg.ntfy.enabled && cfg.ntfy.server_url.is_some() && cfg.ntfy.topic.is_some(),
            };
            if !ready {
                continue;
            }
            let label = match p {
                "native" => "Native",
                "gotify" => "Gotify",
                "telegram" => "Telegram",
                _ => "ntfy",
            };
            let r = platforms::send(p, &title, &message, &cfg, None, None).await;
            lines.push(match (r.suppressed, r.success, r.error) {
                (true, _, _) => format!("— {label}: suppressed (window focused)"),
                (false, true, _) => format!("✓ {label}: sent"),
                (false, false, e) => format!("✗ {label}: {}", e.unwrap_or_else(|| "failed".into())),
            });
        }
        let issues = cfg.validate();
        if !lines.is_empty() && !issues.is_empty() {
            lines.push(format!("Config issues:\n{}", issues.join("\n")));
        }
        if lines.is_empty() {
            let _ = ui::notify(
                "No platforms enabled. Enable one in /unipi:settings (Notify).",
                "warning",
            );
        } else {
            let _ = ui::notify(&format!("Test results:\n{}", lines.join("\n")), "info");
        }
    }

    async fn ask(
        title: &str,
        placeholder: &str,
        prefill: Option<&str>,
        secret: bool,
    ) -> Option<String> {
        let answer = ui::dialog(&json!({
            "method": "input", "title": title, "placeholder": placeholder,
            "prefill": prefill, "secret": secret,
        }))
        .await
        .ok()?;
        answer
            .get("value")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_owned())
    }

    /// Stores a secret field through the settings registry (the host keeps it in the secret
    /// store at the same `notify/<key>` entry); falls back to a direct secret write.
    fn save_secret(&self, key: &str, value: Option<&str>) {
        let written = match value.filter(|v| !v.is_empty()) {
            Some(v) => self
                .settings
                .write(NAMESPACE, key, &json!(v), WriteScope::Global),
            None => self.settings.unset(NAMESPACE, key, WriteScope::Global),
        };
        if written.is_ok() {
            return;
        }
        let name = format!("{NAMESPACE}/{key}");
        match value.filter(|v| !v.is_empty()) {
            Some(v) => {
                let _ = secrets::set(&name, v);
            }
            None => {
                let _ = secrets::delete(&name);
            }
        }
    }

    /// Gotify wizard (unipi `GotifySetupOverlay`): URL → token → priority → test → save.
    async fn gotify_wizard(&self, cx: &CommandCtx) {
        if !cx.has_ui() {
            let _ = ui::notify("Gotify setup requires an interactive UI.", "warning");
            return;
        }
        let cfg = self.config();
        let Some(url) = Self::ask(
            "Gotify server URL",
            "https://gotify.example",
            cfg.gotify.server_url.as_deref(),
            false,
        )
        .await
        .filter(|s| !s.is_empty()) else {
            return;
        };
        let Some(token) = Self::ask(
            "Gotify app token",
            "From the Gotify web UI → Apps",
            cfg.gotify.app_token.as_deref(),
            true,
        )
        .await
        .filter(|s| !s.is_empty()) else {
            return;
        };
        let prio = Self::ask(
            "Gotify priority (1-10)",
            "5",
            Some(&cfg.gotify.priority.to_string()),
            false,
        )
        .await;
        let priority = prio
            .and_then(|p| p.parse::<i64>().ok())
            .filter(|p| (1..=10).contains(p))
            .unwrap_or(5);
        let test = platforms::gotify_request(
            &url,
            &token,
            &format!("{PRODUCT} — Setup Test"),
            &format!("Gotify configured successfully at {}", time_of_day()),
            priority,
        );
        match platforms::post(&test, "Gotify").await {
            Ok(()) => {
                let url = url.trim_end_matches('/').to_owned();
                self.write_setting("gotify.enabled", json!(true));
                self.write_setting("gotify.serverUrl", json!(url));
                self.write_setting("gotify.priority", json!(priority));
                self.save_secret("gotify.appToken", Some(&token));
                let _ = ui::notify("✓ Gotify configured — test message sent.", "info");
            }
            Err(e) => {
                let _ = ui::notify(
                    &format!("Gotify test failed: {e}\nCheck your server URL and app token."),
                    "error",
                );
            }
        }
    }

    /// Telegram wizard (unipi `TelegramSetupOverlay`): token → poll `getUpdates` for the chat id.
    async fn telegram_wizard(&self, cx: &CommandCtx) {
        if !cx.has_ui() {
            let _ = ui::notify("Telegram setup requires an interactive UI.", "warning");
            return;
        }
        let cfg = self.config();
        let Some(token) = Self::ask(
            "Telegram bot token (from @BotFather)",
            "123456:ABC-DEF…",
            cfg.telegram.bot_token.as_deref(),
            true,
        )
        .await
        .map(|t| t.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|s| !s.is_empty()) else {
            return;
        };
        let _ = ui::notify(
            "Now send any message to your bot in Telegram — waiting for it (5 min)…",
            "info",
        );
        let started = timers::now_ms();
        loop {
            if cx.cancel.is_cancelled() {
                return;
            }
            let found = match http::get(&platforms::telegram_updates_url(&token))
                .timeout_ms(15_000)
                .send()
                .await
            {
                Ok(resp) => resp
                    .json::<Value>()
                    .await
                    .ok()
                    .as_ref()
                    .and_then(platforms::chat_id_from_updates),
                Err(_) => None,
            };
            if let Some(chat) = found {
                self.write_setting("telegram.enabled", json!(true));
                self.save_secret("telegram.botToken", Some(&token));
                self.save_secret("telegram.chatId", Some(&chat));
                let _ = ui::notify(&format!("✓ Telegram configured (chat {chat})."), "info");
                return;
            }
            if timers::now_ms() - started > TELEGRAM_POLL_MS {
                let _ = ui::notify("Telegram setup: Timed out after 5 minutes", "error");
                return;
            }
            timers::sleep(Duration::from_secs(2)).await;
        }
    }

    /// ntfy wizard (unipi `NtfySetupOverlay`): server → topic → token → priority → test → save.
    async fn ntfy_wizard(&self, cx: &CommandCtx) {
        if !cx.has_ui() {
            let _ = ui::notify("ntfy setup requires an interactive UI.", "warning");
            return;
        }
        let cfg = self.config();
        let server_default = cfg
            .ntfy
            .server_url
            .clone()
            .unwrap_or_else(|| "https://ntfy.sh".into());
        let Some(server) = Self::ask(
            "ntfy server",
            "https://ntfy.sh",
            Some(&server_default),
            false,
        )
        .await
        .filter(|s| !s.is_empty()) else {
            return;
        };
        let topic_default = cfg
            .ntfy
            .topic
            .clone()
            .unwrap_or_else(|| format!("oino-{}", unipi_kit::fmt::random_hex(10)));
        let Some(topic) = Self::ask(
            "ntfy topic (subscribe to it in the ntfy app)",
            "acts as a channel name",
            Some(&topic_default),
            false,
        )
        .await
        .filter(|s| !s.is_empty()) else {
            return;
        };
        let token = Self::ask(
            "ntfy access token (optional)",
            "leave empty for public topics",
            cfg.ntfy.token.as_deref(),
            true,
        )
        .await;
        let prio = Self::ask(
            "ntfy priority (1-5)",
            "3",
            Some(&cfg.ntfy.priority.to_string()),
            false,
        )
        .await;
        let priority = prio
            .and_then(|p| p.parse::<i64>().ok())
            .filter(|p| (1..=5).contains(p))
            .unwrap_or(3);
        let server = server.trim_end_matches('/').to_owned();
        let body = platforms::ntfy_body(
            Some(&topic),
            &format!("{PRODUCT} — Setup Test"),
            &format!("ntfy configured successfully at {}", time_of_day()),
            priority,
            None,
            false,
        );
        match platforms::post(
            &platforms::ntfy_request(&server, token.as_deref(), body),
            "ntfy",
        )
        .await
        {
            Ok(()) => {
                self.write_setting("ntfy.enabled", json!(true));
                self.write_setting("ntfy.serverUrl", json!(server));
                self.write_setting("ntfy.topic", json!(topic));
                self.write_setting("ntfy.priority", json!(priority));
                self.save_secret("ntfy.token", token.as_deref());
                let _ = ui::notify(
                    &format!("✓ ntfy configured — test sent to {server}/{topic}."),
                    "info",
                );
            }
            Err(e) => {
                let _ = ui::notify(
                    &format!("ntfy test failed: {e}\nCheck your server URL and topic."),
                    "error",
                );
            }
        }
    }

    // ── events ──────────────────────────────────────────────────────────────

    async fn on_timer(&self, ev: &Event) {
        let Some(id) = ev.get("timerId").and_then(Value::as_str) else {
            return;
        };
        let Some((kind, sid)) = parse_timer(id) else {
            return;
        };
        match kind {
            "queue" => self.drain(sid).await,
            "renotify" => self.renotify_tick(sid).await,
            "finish" => {
                if let Some(payload) = self.sessions.with(sid, |s| s.finish_payload.take()) {
                    self.send_finished(sid, &payload).await;
                }
            }
            "alldone" => self.all_done_tick(sid).await,
            _ => {}
        }
    }

    fn on_bus(&self, ev: &Event, cx: &Ctx) {
        let Some(d) = Delivery::from_event(ev) else {
            return;
        };
        let topic = d.topic.as_str();
        if WORK_TOPICS.contains(&topic) {
            self.on_work_change(cx.session_id());
        }
        if cx.is_child() {
            return;
        }
        match topic {
            topics::RALPH_LOOP_END => self.on_builtin(cx, "ralph_loop_end", &d.data),
            topics::MCP_SERVER_ERROR => self.on_builtin(cx, "mcp_server_error", &d.data),
            topics::ASK_USER_PROMPT | topics::foreign::RPIV_ASK_USER_PROMPT => {
                let cfg = self.config();
                if cfg.event("ask_user_prompt").is_some_and(|e| e.enabled) {
                    self.notify_blocking(
                        cx,
                        "ask_user_prompt",
                        messages::ask_user_prompt(&d.data),
                        &cfg,
                    );
                }
            }
            PERMISSION_UI_PROMPT => self.on_builtin(cx, "permission_request", &d.data),
            _ => {}
        }
    }
}

impl Extension for Notify {
    #[allow(clippy::too_many_lines, reason = "one registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = config::namespace();
        cx.settings_namespace(&ns.to_json());
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());

        cx.tool(
            Tool::<Self>::new("notify_user")
                .label("Notify User")
                .description(
                    "Send a notification to the user's configured platforms (native OS, Gotify, Telegram, ntfy). \
                     Use for critical errors, completion of long-running tasks, or when the user explicitly asked to be notified.",
                )
                .parameters(&notify_user_schema())
                .run(|ext, args: NotifyUserArgs, cx| async move {
                    let cfg = ext.config();
                    let platforms: Vec<String> = args.platforms.map_or_else(
                        || cfg.default_platforms.clone(),
                        |p| p.iter().map(|p| p.as_str().to_owned()).collect(),
                    );
                    let priority = args.priority.as_ref().map(Priority::as_str);
                    ext.enqueue(
                        cx.session_id(),
                        Job {
                            title: args
                                .title
                                .filter(|t| !t.is_empty())
                                .unwrap_or_else(|| format!("{PRODUCT} Notification")),
                            message: args.message,
                            platforms: platforms.clone(),
                            event: "agent_tool".into(),
                            priority: priority.map(ToOwned::to_owned),
                        },
                    );
                    let by_platform: serde_json::Map<String, Value> = priority
                        .map(|p| {
                            platforms
                                .iter()
                                .filter_map(|pl| messages::map_priority(pl, p).map(|n| (pl.clone(), json!(n))))
                                .collect()
                        })
                        .unwrap_or_default();
                    Ok(ToolResult::text(format!(
                        "Notification sending to {} platform(s): {}",
                        platforms.len(),
                        platforms.join(", ")
                    ))
                    .with_details(json!({
                        "platforms": platforms,
                        "priority": priority,
                        "priorityByPlatform": priority.map(|_| Value::Object(by_platform)),
                    })))
                }),
        );

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.sessions
                .with(cx.session_id(), |s| *s = SessionState::default());
            if !cx.is_child() {
                ext.settings.refresh();
                ext.import_overlay();
            }
            unipi_kit::bus::subscribe(WORK_TOPICS);
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, ev, cx| async move {
            let sid = cx.session_id().to_owned();
            if !cx.is_child() {
                ext.on_builtin(&cx, "session_shutdown", &ev.payload);
                ext.drain(&sid).await;
            }
            ext.disarm_renotify(&sid);
            ext.disarm_all_done(&sid);
            ext.cancel_finish(&sid);
            ext.sessions.remove(&sid);
            Ok(())
        });
        cx.on(EventKind::AgentStart, |ext, _ev, cx| async move {
            let sid = cx.session_id();
            ext.sessions.with(sid, |s| s.agent_running = true);
            ext.disarm_renotify(sid);
            ext.disarm_all_done(sid);
            ext.cancel_finish(sid);
            Ok(())
        });
        cx.on(EventKind::AgentEnd, |ext, _ev, cx| async move {
            ext.sessions
                .with(cx.session_id(), |s| s.agent_running = false);
            Ok(())
        });
        cx.on(EventKind::AgentSettled, |ext, ev, cx| async move {
            // Children (subagents, sidekicks) never own the user's "finished".
            if cx.is_child() || Self::finish_key(&ext.config()).is_none() {
                return Ok(());
            }
            let sid = cx.session_id();
            ext.cancel_finish(sid);
            ext.sessions
                .with(sid, |s| s.finish_payload = Some(ev.payload.clone()));
            timers::set(
                &timer_id("finish", sid),
                Duration::from_millis(FINISH_GRACE_MS),
                false,
            );
            Ok(())
        });
        cx.on(EventKind::UiPromptStart, |ext, ev, cx| async move {
            if cx.is_child() {
                return Ok(());
            }
            let sid = cx.session_id();
            let (running, last) = ext.sessions.with(sid, |s| {
                s.open_prompts += 1;
                (s.agent_running, s.last_blocking_alert_at)
            });
            let cfg = ext.config();
            if !cfg.event("input_needed").is_some_and(|e| e.enabled) || !running {
                return Ok(());
            }
            if timers::now_ms() - last < PROMPT_DEDUP_MS {
                return Ok(());
            }
            ext.notify_blocking(
                &cx,
                "input_needed",
                messages::input_needed(&ev.payload),
                &cfg,
            );
            Ok(())
        });
        cx.on(EventKind::UiPromptEnd, |ext, _ev, cx| async move {
            let sid = cx.session_id();
            let open = ext.sessions.with(sid, |s| {
                s.open_prompts = s.open_prompts.saturating_sub(1);
                s.open_prompts
            });
            if open == 0 {
                ext.disarm_renotify(sid);
            }
            Ok(())
        });
        cx.on(EventKind::Input, |ext, _ev, cx| async move {
            ext.note_input(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoEditor, |ext, _ev, cx| async move {
            ext.note_input(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            ext.on_bus(&ev, &cx);
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, _cx| async move {
            ext.on_timer(&ev).await;
            Ok(())
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            if ext.settings.on_changed(&ev.payload).is_some() {
                let keys: Vec<String> = ev
                    .get("keys")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                ext.forget_overlay(&keys);
            }
            Ok(())
        });
        unipi_kit::bus::subscribe(&[
            topics::RALPH_LOOP_END,
            topics::MCP_SERVER_ERROR,
            topics::ASK_USER_PROMPT,
            topics::foreign::RPIV_ASK_USER_PROMPT,
            PERMISSION_UI_PROMPT,
        ]);
        unipi_kit::bus::subscribe(WORK_TOPICS);

        cx.command(
            Command::<Self>::new(unipi_kit::suite::command("notify-event"))
                .description("Toggle a notify event without the TUI: <event> <on|off>")
                .argument_hint("<event> <on|off>")
                .run(|ext, args, _cx| async move {
                    ext.notify_event_command(&args);
                    Ok(())
                })
                .complete(|_ext, prefix, _cx| async move {
                    let parts: Vec<&str> = prefix.split_whitespace().collect();
                    let trailing = prefix.ends_with(' ');
                    match (parts.len(), trailing) {
                        (0, _) | (1, false) => {
                            let p = parts.first().copied().unwrap_or("");
                            EVENTS
                                .iter()
                                .filter(|e| e.starts_with(p))
                                .map(|e| item(*e))
                                .collect()
                        }
                        (1, true) | (2, false) => {
                            let p = if trailing { "" } else { parts[1] };
                            ["on", "off"]
                                .iter()
                                .filter(|v| v.starts_with(p))
                                .map(|v| item(format!("{} {v}", parts[0])))
                                .collect()
                        }
                        _ => Vec::new(),
                    }
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:notify-test")
                .description("Send a test notification on every enabled platform")
                .hidden()
                .run(|ext, _args, _cx| async move {
                    ext.test_command().await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:notify-set-gotify")
                .description("Set up Gotify notifications")
                .hidden()
                .run(|ext, _args, cx| async move {
                    ext.gotify_wizard(&cx).await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:notify-set-tg")
                .description("Set up Telegram notifications")
                .hidden()
                .run(|ext, _args, cx| async move {
                    ext.telegram_wizard(&cx).await;
                    Ok(())
                }),
        );
        cx.command(
            Command::<Self>::new("unipi:notify-set-ntfy")
                .description("Set up ntfy notifications")
                .hidden()
                .run(|ext, _args, cx| async move {
                    ext.ntfy_wizard(&cx).await;
                    Ok(())
                }),
        );
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Notify);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_key_removal() {
        let mut v = json!({"events": {"agent_end": {"enabled": true}}, "x": 1});
        assert!(remove_field(&mut v, "events.agent_end.enabled"));
        assert!(!remove_field(&mut v, "events.nope"));
        assert!(remove_field(&mut v, "x"));
        assert_eq!(v, json!({"events": {"agent_end": {}}}));
    }

    #[test]
    fn timer_ids_round_trip() {
        let id = timer_id("renotify", "s:1");
        assert_eq!(id, "notify.renotify:s:1");
        assert_eq!(parse_timer(&id), Some(("renotify", "s:1")));
        assert_eq!(parse_timer("other.x:y"), None);
    }

    #[test]
    fn pending_wake_tasks() {
        assert!(!pending_wake_task(None));
        assert!(!pending_wake_task(Some(
            &json!({"tasks": [{"status": "running", "triggerOnCompletion": false}]})
        )));
        assert!(!pending_wake_task(Some(
            &json!({"tasks": [{"status": "completed", "triggerOnCompletion": true}]})
        )));
        assert!(pending_wake_task(Some(
            &json!({"tasks": [{"status": "running", "triggerOnCompletion": true}]})
        )));
        assert!(!pending_wake_task(Some(&json!({"tasks": "junk"}))));
    }

    #[test]
    fn finish_key_prefers_settled() {
        let mut c = config::from_values(&config::defaults());
        assert_eq!(Notify::finish_key(&c), None);
        c.events
            .iter_mut()
            .for_each(|(k, e)| e.enabled = k == "agent_end");
        assert_eq!(Notify::finish_key(&c), Some("agent_end"));
        c.events.iter_mut().for_each(|(_, e)| e.enabled = true);
        assert_eq!(Notify::finish_key(&c), Some("agent_settled"));
    }
}
