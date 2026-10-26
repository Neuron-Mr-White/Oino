//! `unipi.core` — the extension-level half of unipi's `packages/core` (unipi-port §3.1).
//!
//! | Feature | unipi | Here |
//! |---|---|---|
//! | Unicrab hints | `src/hints/*` | [`hints`]: 132-hint catalog, store `/ext-global/hints.json`, startup/what's-new/event hints, derived detectors, widget above the editor |
//! | start screen | `renderHeaderLines` | `setHeader` with an OUS `responsive` node (≥ 72 / ≥ 40 columns) |
//! | `/unipi:hint [next\|reset]`, browser | `registerCommand("unipi:hint")` | command + overlay panel with filter |
//! | Alt+H / Alt+Shift+H | `registerShortcut` | shortcuts `unipi.hint.next`, `unipi.hint.prev` |
//! | work tray | `src/work/tray.ts` | [`tray`]: `editorReplace` panel over `work-tray.tab` services, `emptyArrowDown` intent, `Shift+↓` (`unipi.tray.open`), hint line below the editor, service `core.open-tray` |
//! | settings | `hints`, `decision-model` | registered namespaces ([`settings`], `unipi_kit::jev`) |
//!
//! Everything else in unipi core (bus, arbiter, command echo, runners, child agents, …) is a host
//! feature or lives in `unipi-kit`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod hints;
pub mod settings;
pub mod tray;

use std::cell::RefCell;
use std::time::Duration;

use oino_extension::command::item;
use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use serde_json::Value;
use unipi_kit::bus::Delivery;
use unipi_kit::services::{OpenTray, fanout as fan, point};
use unipi_kit::settings::Store;
use unipi_kit::tray::{
    Action, ActionReply, TabRequest, TrayTab, decode_tabs, item_count, sorted_visible, split_action,
};
use unipi_kit::{fanout, jev, topics};

use crate::hints::catalog::{Category, Hint};
use crate::hints::engine::{Effect, Engine, Session};
use crate::hints::render::{self, CrabStyle};
use crate::hints::store::{self as hint_store, StoreData};
use crate::settings::{EXTENSION_ID, HINTS, RESET_COMMAND};

/// Topics whose change re-reads the tray tabs.
const TRAY_TOPICS: &[&str] = &[
    topics::WORK_TRAY_CHANGED,
    topics::BG_TASKS,
    topics::SUBAGENTS_LIST,
    topics::FUSION_STATUS,
];

#[derive(Default)]
struct TrayState {
    panel: Option<String>,
    active: Option<String>,
    tabs: Vec<TrayTab>,
}

#[derive(Default)]
struct SessionState {
    hints: Session,
    tray: TrayState,
    browser: Option<(String, String)>,
}

/// The extension.
#[derive(Default)]
pub struct Core {
    settings: Store,
    catalog: RefCell<Vec<Hint>>,
    store: RefCell<Option<StoreData>>,
    sessions: SessionMap<SessionState>,
}

impl Core {
    fn hints_enabled(&self) -> bool {
        self.settings.get(HINTS).bool("enabled", true)
    }

    fn crab_style(&self) -> CrabStyle {
        CrabStyle::parse(self.settings.get(HINTS).str("crab", "auto"))
    }

    /// Runs `f` on the session's hint engine, persists the store and renders the effects.
    fn hints(&self, cx: &Ctx, f: impl FnOnce(&mut Session, &mut Engine<'_>)) {
        let catalog = self.catalog.borrow();
        let mut slot = self.store.borrow_mut();
        let store = slot.get_or_insert_with(|| hint_store::load(&hint_store::path()));
        let mut e = Engine::new(&catalog, store, timers::now_ms());
        e.enabled = cx.has_ui() && self.hints_enabled();
        self.sessions
            .with(cx.session_id(), |s| f(&mut s.hints, &mut e));
        let (dirty, effects) = (e.dirty, std::mem::take(&mut e.effects));
        if dirty {
            hint_store::save(&hint_store::path(), store);
        }
        if !cx.has_ui() {
            return;
        }
        for effect in effects {
            let request = match effect {
                Effect::Show(id) => match catalog.iter().find(|h| h.id == id) {
                    Some(h) => render::widget_request(h, self.crab_style()),
                    None => continue,
                },
                Effect::Clear => render::clear_request(),
            };
            let _ = ui::fire(&request);
        }
    }

    fn show_header(&self, cx: &Ctx) {
        if !cx.has_ui() || !self.settings.get(HINTS).bool("header", true) {
            return;
        }
        let info = host::info();
        let lore: Vec<String> = self
            .catalog
            .borrow()
            .iter()
            .filter(|h| h.category == Category::Lore)
            .map(|h| h.text.clone())
            .collect();
        let pick = u64::from_str_radix(&unipi_kit::fmt::random_hex(8), 16).unwrap_or(0);
        let line = usize::try_from(pick)
            .ok()
            .and_then(|p| lore.get(p % lore.len().max(1)))
            .map_or(render::DEFAULT_LORE, String::as_str);
        let _ = ui::fire(&render::header_request(
            &info.extension_version,
            &info.oino_version,
            line,
            self.crab_style(),
        ));
    }

    fn reset(&self, cx: &CommandCtx) {
        let mut slot = self.store.borrow_mut();
        let store = slot.get_or_insert_with(|| hint_store::load(&hint_store::path()));
        store.reset();
        hint_store::save(&hint_store::path(), store);
        if cx.has_ui() {
            let _ = ui::notify(
                "Hint history cleared — every hint is eligible again.",
                "info",
            );
        }
    }

    fn open_browser(&self, cx: &Ctx, query: &str) -> Result<()> {
        if !cx.has_ui() {
            return Ok(());
        }
        let catalog = self.catalog.borrow();
        let store = self
            .store
            .borrow_mut()
            .get_or_insert_with(|| hint_store::load(&hint_store::path()))
            .clone();
        let existing = self.sessions.with(cx.session_id(), |s| {
            s.browser.as_ref().map(|(id, _)| id.clone())
        });
        let id = match existing {
            Some(id) => {
                ui::panel_update(
                    &id,
                    &serde_json::json!({"spec": render::browser_spec(&catalog, &store, query)}),
                )?;
                id
            }
            None => ui::panel_open(&render::browser_panel(&catalog, &store, query))?,
        };
        self.sessions.with(cx.session_id(), |s| {
            s.browser = Some((id, query.to_owned()));
        });
        Ok(())
    }

    fn hint_command(&self, args: &str, cx: &CommandCtx) -> Result<()> {
        match args.trim() {
            "next" => self.hints(&cx.cx, hints::engine::Session::cycle_next),
            "reset" => self.reset(cx),
            _ => self.open_browser(&cx.cx, "")?,
        }
        Ok(())
    }

    // ── work tray ────────────────────────────────────────────────────────────

    async fn fetch_tabs(open: bool, initial_id: Option<String>) -> Vec<TrayTab> {
        let request = serde_json::to_value(TabRequest { open, initial_id }).unwrap_or(Value::Null);
        let replies = fanout::call_all(
            fan::WORK_TRAY_TAB,
            &request,
            fanout::DEFAULT_TIMEOUT,
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

    async fn refresh_tray(&self, cx: &Ctx) {
        if !cx.has_ui() {
            return;
        }
        let open = self
            .sessions
            .with(cx.session_id(), |s| s.tray.panel.is_some());
        let tabs = Self::fetch_tabs(open, None).await;
        let (panel, active) = self.sessions.with(cx.session_id(), |s| {
            s.tray.tabs.clone_from(&tabs);
            (s.tray.panel.clone(), s.tray.active.clone())
        });
        if let Some(id) = &panel {
            let i = tray::active_index(&tabs, active.as_deref());
            let _ = ui::panel_update(id, &serde_json::json!({"spec": tray::spec(&tabs, i)}));
        }
        let _ = ui::fire(&tray::strip_request(&tabs, panel.is_some()));
    }

    async fn open_tray(&self, cx: &Ctx, request: OpenTray) -> bool {
        if !cx.has_ui() {
            return false;
        }
        let tabs = Self::fetch_tabs(true, request.initial_id).await;
        if tabs.is_empty() {
            return false;
        }
        let (panel, remembered) = self.sessions.with(cx.session_id(), |s| {
            (s.tray.panel.clone(), s.tray.active.clone())
        });
        let i = tray::active_index(&tabs, request.tab.as_deref().or(remembered.as_deref()));
        let id = match panel {
            Some(id) => ui::panel_update(&id, &serde_json::json!({"spec": tray::spec(&tabs, i)}))
                .ok()
                .map(|()| id),
            None => ui::panel_open(&tray::open_request(&tabs, i)).ok(),
        };
        let Some(id) = id else { return false };
        let active = tabs.get(i).map(|t| t.id.clone());
        self.sessions.with(cx.session_id(), |s| {
            s.tray = TrayState {
                panel: Some(id),
                active,
                tabs: tabs.clone(),
            };
        });
        let _ = ui::fire(&tray::strip_request(&tabs, true));
        true
    }

    async fn tray_action(&self, cx: &Ctx, action_id: &str, value: Option<Value>) {
        let sid = cx.session_id().to_owned();
        match action_id {
            "$close" => {
                let tabs = self.sessions.with(&sid, |s| {
                    s.tray.panel = None;
                    s.tray.tabs.clone()
                });
                let _ = ui::fire(&tray::strip_request(&tabs, false));
            }
            tray::TAB_ACTION => {
                let tab = value
                    .as_ref()
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.sessions.with(&sid, |s| s.tray.active = tab);
                self.refresh_tray(cx).await;
            }
            other => {
                let Some((tab, action)) = split_action(other) else {
                    return;
                };
                let owner = self.sessions.with(&sid, |s| {
                    s.tray
                        .tabs
                        .iter()
                        .find(|t| t.id == tab)
                        .and_then(|t| t.owner.clone())
                });
                let Some(owner) = owner else { return };
                let payload = serde_json::to_value(Action {
                    tab: tab.to_owned(),
                    action_id: action.to_owned(),
                    value,
                })
                .unwrap_or(Value::Null);
                let reply = services::call(
                    Some(&owner),
                    point::WORK_TRAY_ACTION,
                    &payload,
                    Some(Duration::from_secs(5)),
                )
                .await
                .ok()
                .and_then(unipi_kit::services::decode::<ActionReply>)
                .unwrap_or_default();
                if reply.close {
                    let panel = self.sessions.with(&sid, |s| s.tray.panel.take());
                    if let Some(id) = panel {
                        ui::panel_close(&id);
                    }
                    let tabs = self.sessions.with(&sid, |s| s.tray.tabs.clone());
                    let _ = ui::fire(&tray::strip_request(&tabs, false));
                } else if reply.refresh {
                    self.refresh_tray(cx).await;
                }
            }
        }
    }

    fn browser_action(&self, cx: &Ctx, action_id: &str, value: Option<&Value>) -> Result<()> {
        match action_id {
            "$close" => self.sessions.with(cx.session_id(), |s| s.browser = None),
            "filter" => self.open_browser(cx, value.and_then(Value::as_str).unwrap_or(""))?,
            "show" => {
                let Some(id) = value.and_then(Value::as_str).map(ToOwned::to_owned) else {
                    return Ok(());
                };
                let panel = self.sessions.with(cx.session_id(), |s| s.browser.take());
                if let Some((pid, _)) = panel {
                    ui::panel_close(&pid);
                }
                self.hints(cx, |s, e| s.show(e, &id, true));
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_ui_action(&self, ev: &Event, cx: &Ctx) -> Result<()> {
        let panel = ev
            .get("panelId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let action = ev
            .get("actionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let value = ev.get("value").cloned();
        let (tray_id, browser_id) = self.sessions.with(cx.session_id(), |s| {
            (
                s.tray.panel.clone(),
                s.browser.as_ref().map(|(id, _)| id.clone()),
            )
        });
        if Some(&panel) == tray_id.as_ref() || panel == tray::PANEL {
            self.tray_action(cx, &action, value).await;
        } else if Some(&panel) == browser_id.as_ref() || panel == render::BROWSER_PANEL {
            self.browser_action(cx, &action, value.as_ref())?;
        }
        Ok(())
    }

    async fn on_editor(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        if ev.get("intent").and_then(Value::as_str) != Some("emptyArrowDown") {
            return None;
        }
        let (open, count) = self.sessions.with(cx.session_id(), |s| {
            (s.tray.panel.is_some(), item_count(&s.tray.tabs))
        });
        if open || count == 0 {
            return None;
        }
        self.open_tray(cx, OpenTray::default())
            .await
            .then(|| serde_json::json!({"consume": true}))
    }

    // ── events ───────────────────────────────────────────────────────────────

    async fn on_session_start(&self, cx: &Ctx) {
        self.settings.refresh();
        self.sessions.remove(cx.session_id());
        self.show_header(cx);
        let version = host::info().extension_version;
        self.hints(cx, |s, e| s.start(e, &version));
        self.refresh_tray(cx).await;
    }

    async fn on_bus(&self, ev: &Event, cx: &Ctx) {
        let Some(d) = Delivery::from_event(ev) else {
            return;
        };
        if TRAY_TOPICS.contains(&d.topic.as_str()) {
            self.refresh_tray(cx).await;
        }
        if topics::is_unipi_topic(&d.topic) {
            self.hints(cx, |s, e| s.fire(e, &d.topic, &d.data));
        }
    }

    fn on_turn_end(&self, cx: &Ctx) {
        let percent = control::context_usage()
            .ok()
            .and_then(|u| u.get("percent").and_then(Value::as_f64));
        let usage = session::leaf_id()
            .and_then(|id| session::entry(&id).ok().flatten())
            .and_then(|entry| {
                let m = entry.get("message")?;
                if entry.get("type")?.as_str()? != "message"
                    || m.get("role")?.as_str()? != "assistant"
                {
                    return None;
                }
                let u = m.get("usage")?;
                Some((
                    u.get("input").and_then(Value::as_f64).unwrap_or(0.0),
                    u.get("cacheRead").and_then(Value::as_f64).unwrap_or(0.0),
                ))
            });
        // The host exposes no local time zone yet: late-night stays off (see deviations).
        self.hints(cx, |s, e| s.on_turn_end(e, percent, None, usage));
    }

    fn on_pi_event(&self, ev: &Event, cx: &Ctx) {
        let name = oino_extension::events::wire_name(ev.kind);
        let str_field = |k: &str| {
            ev.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match ev.kind {
            EventKind::Input => {
                let text = str_field("text");
                let images = ev
                    .get("images")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                self.hints(cx, |s, e| s.on_input(e, &text, images));
            }
            EventKind::BeforeAgentStart => {
                self.hints(cx, hints::engine::Session::before_agent_start);
            }
            EventKind::TurnEnd => self.on_turn_end(cx),
            EventKind::ToolExecutionStart => {
                let (tool, id) = (str_field("toolName"), str_field("toolCallId"));
                self.sessions.with(cx.session_id(), |s| {
                    s.hints.on_tool_start(&tool, &id, timers::now_ms());
                });
            }
            EventKind::ToolExecutionEnd => {
                let (tool, id) = (str_field("toolName"), str_field("toolCallId"));
                self.hints(cx, |s, e| s.on_tool_end(e, &tool, &id));
            }
            EventKind::ToolResult => {
                let is_error = ev.get("isError").and_then(Value::as_bool).unwrap_or(false);
                self.hints(cx, |s, e| s.on_tool_result(e, is_error));
            }
            EventKind::ToolCall => {
                let tool = str_field("toolName");
                let input = ev.get("input").cloned().unwrap_or(Value::Null);
                self.hints(cx, |s, e| s.on_tool_call(e, &tool, &input));
            }
            _ => {}
        }
        self.hints(cx, |s, e| s.fire(e, &name, &ev.payload));
    }
}

/// pi events the hint engine observes (derived detectors + event hints).
const PI_EVENTS: &[EventKind] = &[
    EventKind::Input,
    EventKind::BeforeAgentStart,
    EventKind::TurnEnd,
    EventKind::ToolExecutionStart,
    EventKind::ToolExecutionEnd,
    EventKind::ToolResult,
    EventKind::ToolCall,
    EventKind::UserBash,
    EventKind::ModelSelect,
    EventKind::SessionCompact,
    EventKind::SessionCompactFailed,
];

impl Extension for Core {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        *self.catalog.borrow_mut() = hints::catalog::bundled()?;
        let hints_ns = settings::hints_namespace();
        let decision = jev::namespace(EXTENSION_ID);
        self.settings.declare(HINTS, hints_ns.defaults.clone());
        self.settings
            .declare(jev::NAMESPACE, decision.defaults.clone());
        self.settings.load(cx.settings());
        cx.settings_namespace(&hints_ns.to_json());
        cx.settings_namespace(&decision.to_json());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.on_session_start(&cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
        for kind in PI_EVENTS {
            cx.on(*kind, |ext, ev, cx| async move {
                ext.on_pi_event(&ev, &cx);
                Ok(())
            });
        }
        cx.on(EventKind::OinoBus, |ext, ev, cx| async move {
            ext.on_bus(&ev, &cx).await;
            Ok(())
        });
        cx.on(EventKind::OinoEditor, |ext, ev, cx| async move {
            Ok(ext.on_editor(&ev, &cx).await)
        });
        cx.on(EventKind::OinoUiAction, |ext, ev, cx| async move {
            ext.on_ui_action(&ev, &cx).await
        });
        cx.on(EventKind::OinoSettingsChanged, |ext, ev, cx| async move {
            if ext.settings.on_changed(&ev.payload).as_deref() == Some(HINTS)
                && !ext.hints_enabled()
                && cx.has_ui()
            {
                let _ = ui::fire(&render::clear_request());
            }
            Ok(())
        });
        bus::subscribe("unipi:*");

        cx.command(
            Command::<Self>::new(unipi_kit::suite::command("hint"))
                .description("Browse and search Unicrab hints, or cycle/reset")
                .argument_hint("[next|reset]")
                .run(|ext, args, cx| async move { ext.hint_command(&args, &cx) })
                .complete(|_ext, prefix, _cx| async move {
                    ["next", "reset"]
                        .into_iter()
                        .filter(|v| v.starts_with(prefix.trim()))
                        .map(item)
                        .collect()
                }),
        );
        cx.command(
            Command::<Self>::new(RESET_COMMAND)
                .description("Reset Unicrab hint history")
                .hidden()
                .run(|ext, _args, cx| async move {
                    ext.reset(&cx);
                    Ok(())
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.hint.next", &["alt+h"])
                .description("Next hint (Unicrab)")
                .run(|ext, cx| async move {
                    ext.hints(&cx.cx, hints::engine::Session::cycle_next);
                    Ok(())
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.hint.prev", &["alt+shift+h"])
                .description("Previous hint (Unicrab)")
                .run(|ext, cx| async move {
                    ext.hints(&cx.cx, hints::engine::Session::cycle_prev);
                    Ok(())
                }),
        );
        cx.shortcut(
            Shortcut::<Self>::new("unipi.tray.open", &["shift+down"])
                .description("Open the work tray")
                .run(|ext, cx| async move {
                    ext.open_tray(&cx.cx, OpenTray::default()).await;
                    Ok(())
                }),
        );
        cx.service(
            point::CORE_OPEN_TRAY,
            |ext, payload, _caller, cx| async move {
                let request = unipi_kit::services::decode::<OpenTray>(payload).unwrap_or_default();
                Ok(serde_json::json!({"opened": ext.open_tray(&cx, request).await}))
            },
        );
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports: their `:`-qualified symbol names are
// not valid in host linker version scripts.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Core);
