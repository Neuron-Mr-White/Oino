//! `unipi.app-bridge` — the session control centre for the Oino GPUI apps (unipi
//! `packages/app-bridge`, re-targeted per gpui-app.md).
//!
//! unipi's app-bridge mirrored a TUI session to the `UniPi` phone app over a unix socket. In
//! Oino the frontend-agnostic RPC/serve surface *is* the mirror (multi-client attach,
//! first-answer-wins dialogs, session operations, `get_completions`, files), so those wire
//! cases are host features. What remains extension-level is the UNI-160 "session control
//! centre": unipi-specific state gathered from the other packages and their actions. Here it
//! is a `sidePane` panel (GPUI desktop side-pane tab; mobile bottom sheet; TUI overlay) plus
//! `app-bridge.snapshot` / `app-bridge.action` services carrying the same data as JSON.
//!
//! | unipi wire case | here |
//! |---|---|
//! | `hello`/`state`/`stats`/`watch{stats}` (run state, model, thinking, context, cost, tokens, tps) | "Session" tab, refreshed ≤ 1/s on events |
//! | `set_model`, `set_thinking`, `compact`, `abort` | tab controls (`control.*`) |
//! | `work_*` (list, stop, background, log, transcript) | "Work" tab over the `work-tray.tab` fan-out; actions forwarded to the owner (`work-tray.action`) |
//! | `set_lh_mode`, `lh_progress_get`, `lh_graph` | "Modes" tab: mode chip → hidden command `lh-mode <mode>`, progress from `unipi:long-horizon:progress` |
//! | `set_fusion` | "Change pair…" → `/fusion` (fusion's picker) |
//! | `dream_get`, `dream_action` (run/stop) | `dream.status` / `dream.run` / `dream.stop` |
//! | `btw`, `btw_list` | "BTW" tab → `btw.ask` |
//! | `watch{info}` | "Info" tab over the `info-screen.groups` fan-out |
//! | prompt/abort/answer/queue_*/history/tree/sessions/session_*/reload/files/media/search/paths_stat/arg_completions/file_share | host (RPC/serve, ui-protocol §3, modes §4) |

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

mod view;

use std::time::Duration;

use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use unipi_kit::bus::Delivery;
use unipi_kit::services::{fanout as fan, point};
use unipi_kit::settings::{Field, Namespace, Section, Store};
use unipi_kit::topics;
use unipi_kit::tray::{
    Action, ActionReply, TabRequest, TrayTab, decode_tabs, sorted_visible, split_action,
};

use crate::view::{EXTENSION_ID, Stats};

const NAMESPACE: &str = "app-bridge";
const REFRESH_MS: u64 = 1000;
const SNAPSHOT_SERVICE: &str = "app-bridge.snapshot";
const ACTION_SERVICE: &str = "app-bridge.action";
const LH_MODE_COMMAND: &str = "lh-mode";
const FUSION_PICKER_COMMAND: &str = "fusion";
const TABS: [(&str, &str); 5] = [
    ("session", "Session"),
    ("work", "Work"),
    ("modes", "Modes"),
    ("btw", "BTW"),
    ("info", "Info"),
];
/// Topics that change what the control centre shows.
const WATCHED: &[&str] = &[
    topics::LH_STATE,
    topics::LH_PROGRESS,
    topics::FUSION_STATUS,
    topics::WORKFLOW_STATUS,
    topics::KANBOARD_STATUS,
    topics::BG_TASKS,
    topics::SUBAGENTS_LIST,
    topics::FOOTER_TPS,
    topics::WORK_TRAY_CHANGED,
];

#[derive(Default)]
struct SessionState {
    panel: Option<String>,
    active: String,
    btw: Vec<(String, String)>,
    btw_queue: Vec<String>,
    refresh_armed: bool,
    tabs: Vec<TrayTab>,
}

/// The extension.
#[derive(Default)]
pub struct AppBridge {
    settings: Store,
    sessions: SessionMap<SessionState>,
}

fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "App bridge", EXTENSION_ID)
        .defaults(json!({"enabled": true, "autoOpen": true}))
        .section(
            Section::new("control", "Control centre")
                .field(Field::boolean("enabled", "Enabled").description("Offer the session control centre (/app)."))
                .field(
                    Field::boolean("autoOpen", "Open in the app side pane")
                        .description("Open the control centre as a side-pane tab when Oino Desktop or Oino Mobile is attached."),
                ),
        )
}

fn timer_id(sid: &str) -> String {
    format!("app-bridge.refresh:{sid}")
}

/// GPUI frontends (gpui-app.md §3, §4).
fn gpui_attached(frontends: &[String]) -> bool {
    frontends.iter().any(|f| f == "desktop" || f == "mobile")
}

fn answer_text(v: &Value) -> String {
    v.as_str()
        .map(ToOwned::to_owned)
        .or_else(|| {
            v.get("answer")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| v.get("text").and_then(Value::as_str).map(ToOwned::to_owned))
        .unwrap_or_else(|| serde_json::to_string_pretty(v).unwrap_or_default())
}

impl AppBridge {
    fn enabled(&self) -> bool {
        self.settings.get(NAMESPACE).bool("enabled", true)
    }

    fn stats() -> Stats {
        let model = control::model();
        let key = model.as_ref().and_then(|m| {
            Some(format!(
                "{}/{}",
                m.get("provider")?.as_str()?,
                m.get("id")?.as_str()?
            ))
        });
        let entries = session::branch(None).unwrap_or_default();
        let (tokens_in, tokens_out, cache_hit, cost) = view::usage_totals(&entries);
        let tps = unipi_kit::bus::get_raw(topics::FOOTER_TPS)
            .and_then(|t| t.get("sessionAvgTps").and_then(Value::as_f64))
            .unwrap_or(0.0);
        Stats {
            run_state: control::run_state()
                .ok()
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| {
                    if control::is_idle() {
                        "idle"
                    } else {
                        "running"
                    }
                    .to_owned()
                }),
            model_name: model
                .as_ref()
                .and_then(|m| m.get("name"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            reasoning: model
                .as_ref()
                .and_then(|m| m.get("reasoning"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            model: key,
            thinking: control::thinking_level(),
            context: control::context_usage()
                .map(|c| view::context_of(&c))
                .unwrap_or_default(),
            tokens_in,
            tokens_out,
            cache_hit,
            cost,
            tps: (tps * 10.0).round() / 10.0,
        }
    }

    async fn work_tabs() -> Vec<TrayTab> {
        let req = serde_json::to_value(TabRequest {
            open: true,
            initial_id: None,
        })
        .unwrap_or(Value::Null);
        let replies = unipi_kit::fanout::call_all(
            fan::WORK_TRAY_TAB,
            &req,
            unipi_kit::fanout::DEFAULT_TIMEOUT,
            Some(EXTENSION_ID),
        )
        .await;
        sorted_visible(
            replies
                .iter()
                .flat_map(|r| decode_tabs(&r.extension, &r.value))
                .collect(),
        )
    }

    async fn info_groups() -> Vec<Value> {
        unipi_kit::fanout::call_all(
            fan::INFO_SCREEN_GROUPS,
            &json!({}),
            unipi_kit::fanout::DEFAULT_TIMEOUT,
            Some(EXTENSION_ID),
        )
        .await
        .into_iter()
        .flat_map(|r| match r.value {
            Value::Array(a) => a,
            v => vec![v],
        })
        .collect()
    }

    async fn dream() -> Option<Value> {
        services::call(
            Some("unipi.dream"),
            point::DREAM_STATUS,
            &json!({}),
            Some(Duration::from_secs(2)),
        )
        .await
        .ok()
        .filter(|v| !v.is_null())
    }

    fn models() -> Vec<Value> {
        control::models()
            .unwrap_or_default()
            .into_iter()
            .take(200)
            .collect()
    }

    /// Builds and shows (or updates) the panel.
    async fn render(&self, cx: &Ctx, open: bool) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        let sid = cx.session_id();
        let (panel, active, btw, pending) = self.sessions.with(sid, |s| {
            if s.active.is_empty() {
                "session".clone_into(&mut s.active);
            }
            (
                s.panel.clone(),
                s.active.clone(),
                s.btw.clone(),
                !s.btw_queue.is_empty(),
            )
        });
        if panel.is_none() && !open {
            return Ok(());
        }
        let stats = Self::stats();
        let waiting = (stats.run_state == "waiting").then_some("background work");
        let tabs = Self::work_tabs().await;
        let info = view::info_rows(&Self::info_groups().await);
        let dream = Self::dream().await;
        let lh = unipi_kit::bus::get_raw(topics::LH_STATE);
        let progress = unipi_kit::bus::get_raw(topics::LH_PROGRESS);
        let workflow = unipi_kit::bus::get_raw(topics::WORKFLOW_STATUS);
        let fusion = unipi_kit::bus::get_raw(topics::FUSION_STATUS);
        let bodies = vec![
            (
                TABS[0].0,
                TABS[0].1,
                view::session_tab(&stats, &Self::models(), waiting),
            ),
            (TABS[1].0, TABS[1].1, view::work_tab(&tabs)),
            (
                TABS[2].0,
                TABS[2].1,
                view::modes_tab(
                    lh.as_ref(),
                    progress.as_ref(),
                    workflow.as_ref(),
                    fusion.as_ref(),
                    dream.as_ref(),
                ),
            ),
            (TABS[3].0, TABS[3].1, view::btw_tab(&btw, pending)),
            (TABS[4].0, TABS[4].1, view::info_tab(&info)),
        ];
        let spec = view::panel_spec(&active, bodies);
        self.sessions.with(sid, |s| s.tabs.clone_from(&tabs));
        if let Some(id) = panel {
            ui::panel_update(&id, &json!({"spec": spec}))?;
        } else {
            let id = ui::panel_open(&view::open_request(&spec))?;
            self.sessions.with(sid, |s| s.panel = Some(id));
        }
        Ok(())
    }

    /// Throttled refresh (unipi `schedulePush`, ≤ 1/s).
    fn schedule(&self, sid: &str) {
        let arm = self.sessions.with(sid, |s| {
            let arm = s.panel.is_some() && !s.refresh_armed;
            if arm {
                s.refresh_armed = true;
            }
            arm
        });
        if arm {
            timers::set(&timer_id(sid), Duration::from_millis(REFRESH_MS), false);
        }
    }

    /// Runs one control-centre action; `Ok(message)` is shown to the user when non-empty.
    #[allow(
        clippy::too_many_lines,
        reason = "one match over every control-centre action"
    )]
    async fn act(
        &self,
        cx: &Ctx,
        action: &str,
        value: Option<&Value>,
    ) -> std::result::Result<String, String> {
        let text = value.and_then(Value::as_str).unwrap_or("").to_owned();
        match action {
            "refresh" => Ok(String::new()),
            "set_model" => {
                let (provider, id) = text.split_once('/').ok_or("model must be provider/id")?;
                match control::set_model(provider, id) {
                    Ok(true) => Ok(String::new()),
                    Ok(false) => Err(format!("No credentials for {text}.")),
                    Err(e) => Err(e.to_string()),
                }
            }
            "set_thinking" => control::set_thinking_level(&text)
                .map(|_| String::new())
                .map_err(|e| e.to_string()),
            "compact" => control::compact(None)
                .await
                .map(|_| "Compacted.".to_owned())
                .map_err(|e| e.to_string()),
            "abort" => {
                control::abort();
                Ok(String::new())
            }
            "set_lh_mode" => {
                if !view::LH_MODES.contains(&text.as_str()) {
                    return Err("set_lh_mode.mode must be goal|ralph|swarm|graph|none".into());
                }
                match control::invoke_command(LH_MODE_COMMAND, &text).await {
                    Ok(Some(v)) if v.get("ok").and_then(Value::as_bool) == Some(false) => Err(v
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("Could not switch the mode.")
                        .to_owned()),
                    Ok(_) => Ok(String::new()),
                    Err(_) => Err(
                        "Long-horizon modes are not available (unipi.long-horizon is not enabled)."
                            .into(),
                    ),
                }
            }
            "fusion_pick" => control::invoke_command(FUSION_PICKER_COMMAND, "")
                .await
                .map(|_| String::new())
                .map_err(|_| "Fusion is not available (unipi.fusion is not enabled).".to_owned()),
            "dream_run" | "dream_stop" => {
                let name = if action == "dream_run" {
                    point::DREAM_RUN
                } else {
                    point::DREAM_STOP
                };
                services::call(
                    Some("unipi.dream"),
                    name,
                    &json!({}),
                    Some(Duration::from_secs(5)),
                )
                .await
                .map(|_| String::new())
                .map_err(|_| "Dream is not available (unipi.dream is not enabled).".to_owned())
            }
            "btw_ask" => {
                if text.trim().is_empty() {
                    return Ok(String::new());
                }
                self.sessions.with(cx.session_id(), |s| {
                    s.btw_queue.push(text.trim().to_owned());
                });
                timers::set(
                    &format!("app-bridge.btw:{}", cx.session_id()),
                    Duration::ZERO,
                    false,
                );
                Ok(String::new())
            }
            other => {
                let Some((tab, rest)) = split_action(other) else {
                    return Err(format!("Unknown action \"{other}\"."));
                };
                let owner = self.sessions.with(cx.session_id(), |s| {
                    s.tabs
                        .iter()
                        .find(|t| t.id == tab)
                        .and_then(|t| t.owner.clone())
                });
                let Some(owner) = owner else {
                    return Err(format!("No work tab \"{tab}\"."));
                };
                let a = Action {
                    tab: tab.to_owned(),
                    action_id: rest.to_owned(),
                    value: value.cloned(),
                };
                let reply = services::call(
                    Some(&owner),
                    point::WORK_TRAY_ACTION,
                    &serde_json::to_value(&a).unwrap_or(Value::Null),
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
                let _: ActionReply = unipi_kit::services::decode(reply).unwrap_or_default();
                Ok(String::new())
            }
        }
    }

    async fn on_ui_action(&self, ev: &Event, cx: &Ctx) -> Result<()> {
        let Some(panel) = ev.get("panelId").and_then(Value::as_str) else {
            return Ok(());
        };
        if self
            .sessions
            .with(cx.session_id(), |s| s.panel.as_deref() != Some(panel))
        {
            return Ok(());
        }
        let action = ev.get("actionId").and_then(Value::as_str).unwrap_or("");
        match action {
            "$close" => {
                self.sessions.with(cx.session_id(), |s| s.panel = None);
                return Ok(());
            }
            "$tab" => {
                if let Some(t) = ev.get("value").and_then(Value::as_str) {
                    self.sessions
                        .with(cx.session_id(), |s| t.clone_into(&mut s.active));
                }
            }
            _ => match self.act(cx, action, ev.get("value")).await {
                Ok(msg) if !msg.is_empty() => {
                    let _ = ui::notify(&msg, "info");
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = ui::notify(&e, "warning");
                }
            },
        }
        self.render(cx, false).await
    }

    async fn answer_btw(&self, cx: &Ctx) {
        let sid = cx.session_id().to_owned();
        let questions = self.sessions.with(&sid, |s| s.btw_queue.clone());
        if questions.is_empty() {
            return;
        }
        let _ = self.render(cx, false).await;
        for q in questions {
            let a = match services::call(
                Some("unipi.btw"),
                point::BTW_ASK,
                &json!({"question": q}),
                Some(Duration::from_secs(300)),
            )
            .await
            {
                Ok(v) => answer_text(&v),
                Err(e) => format!("_btw is not available: {e}_"),
            };
            self.sessions.with(&sid, |s| {
                s.btw_queue.retain(|x| *x != q);
                s.btw.push((q.clone(), a));
                if s.btw.len() > 20 {
                    s.btw.remove(0);
                }
            });
        }
        let _ = self.render(cx, false).await;
    }

    async fn snapshot(&self) -> Value {
        let tabs = Self::work_tabs().await;
        let work: Vec<Value> = tabs
            .iter()
            .map(|t| json!({"id": t.id, "label": t.label, "owner": t.owner, "counts": {"total": t.counts.total, "running": t.counts.running}}))
            .collect();
        let info: Vec<Value> = view::info_rows(&Self::info_groups().await)
            .into_iter()
            .map(|(name, stats)| json!({"label": name, "stats": stats.into_iter().map(|(l, v)| json!({"label": l, "value": v})).collect::<Vec<_>>()}))
            .collect();
        json!({
            "state": Self::stats(),
            "lhMode": unipi_kit::bus::get_raw(topics::LH_STATE),
            "lhProgress": unipi_kit::bus::get_raw(topics::LH_PROGRESS),
            "workflow": unipi_kit::bus::get_raw(topics::WORKFLOW_STATUS),
            "fusion": unipi_kit::bus::get_raw(topics::FUSION_STATUS),
            "kanboard": unipi_kit::bus::get_raw(topics::KANBOARD_STATUS),
            "dream": Self::dream().await,
            "work": work,
            "info": info,
        })
    }
}

impl Extension for AppBridge {
    #[allow(clippy::too_many_lines, reason = "one registration list")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let ns = namespace();
        cx.settings_namespace(&ns.to_json());
        self.settings.declare(NAMESPACE, ns.defaults.clone());
        self.settings.load(cx.settings());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            // Never in a child session (unipi app-bridge rule).
            if cx.is_child() || !cx.has_ui() || !ext.enabled() {
                return Ok(());
            }
            if ext.settings.get(NAMESPACE).bool("autoOpen", true)
                && gpui_attached(&host::info().frontends)
            {
                ext.render(&cx, true).await?;
            }
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            timers::cancel(&timer_id(cx.session_id()));
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        for kind in [
            EventKind::AgentStart,
            EventKind::AgentEnd,
            EventKind::TurnEnd,
            EventKind::ModelSelect,
            EventKind::ThinkingLevelSelect,
            EventKind::SessionCompact,
            EventKind::MessageEnd,
        ] {
            cx.on(kind, |ext, _ev, cx| async move {
                ext.schedule(cx.session_id());
                Ok(())
            });
        }
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            if Delivery::from_event(&ev).is_some_and(|d| WATCHED.contains(&d.topic.as_str())) {
                ext.schedule(cx.session_id());
            }
            Ok(())
        });
        cx.on(EventKind::OinoTimer, |ext, ev, cx| async move {
            let id = ev.get("timerId").and_then(Value::as_str).unwrap_or("");
            if id == timer_id(cx.session_id()) {
                ext.sessions
                    .with(cx.session_id(), |s| s.refresh_armed = false);
                let _ = ext.render(&cx, false).await;
            } else if id.starts_with("app-bridge.btw:") {
                ext.answer_btw(&cx).await;
            }
            Ok(())
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_ui_action(&ev, &cx).await
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, _cx| async move {
            let _ = ext.settings.on_changed(&ev.payload);
            Ok(())
        });
        unipi_kit::bus::subscribe(WATCHED);

        cx.command(
            Command::<Self>::new("app")
                .description("Open the session control centre (Oino Desktop/Mobile side pane)")
                .argument_hint("[session|work|modes|btw|info]")
                .run(|ext, args, cx| async move {
                    if !cx.has_ui() {
                        return Ok(());
                    }
                    if !ext.enabled() {
                        let _ =
                            ui::notify("The control centre is off (app-bridge.enabled).", "info");
                        return Ok(());
                    }
                    let tab = args.trim();
                    if TABS.iter().any(|(id, _)| *id == tab) {
                        ext.sessions
                            .with(cx.session_id(), |s| tab.clone_into(&mut s.active));
                    }
                    ext.render(&cx.cx, true).await
                })
                .complete(|_ext, prefix, _cx| async move {
                    TABS.iter()
                        .filter(|(id, _)| id.starts_with(prefix.trim()))
                        .map(|(id, _)| oino_extension::command::item(*id))
                        .collect()
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.app.open", &[])
                .description("Open the session control centre")
                .run(|ext, cx| async move { ext.render(&cx.cx, true).await }),
        );
        cx.service(SNAPSHOT_SERVICE, |ext, _p, _caller, _cx| async move {
            Ok(ext.snapshot().await)
        });
        cx.service(ACTION_SERVICE, |ext, p, _caller, cx| async move {
            let action = p
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let r = ext.act(&cx, &action, p.get("value")).await;
            ext.schedule(cx.session_id());
            Ok(match r {
                Ok(message) => json!({"ok": true, "message": message}),
                Err(message) => json!({"ok": false, "message": message}),
            })
        });
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(AppBridge);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpui_detection() {
        assert!(gpui_attached(&["tui".into(), "desktop".into()]));
        assert!(gpui_attached(&["mobile".into()]));
        assert!(!gpui_attached(&["tui".into(), "rpc".into()]));
    }

    #[test]
    fn btw_answers() {
        assert_eq!(answer_text(&json!("plain")), "plain");
        assert_eq!(answer_text(&json!({"answer": "a"})), "a");
        assert_eq!(answer_text(&json!({"text": "t"})), "t");
        assert!(answer_text(&json!({"x": 1})).contains("\"x\": 1"));
    }

    #[test]
    fn namespace_shape() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "app-bridge");
        assert_eq!(v["defaults"], json!({"enabled": true, "autoOpen": true}));
        assert_eq!(timer_id("s"), "app-bridge.refresh:s");
    }
}
