//! `unipi.core` — the extension-level half of unipi's `packages/core` (unipi-port §3.1).
//!
//! | Feature | unipi | Here |
//! |---|---|---|
//! | work tray | `src/work/tray.ts` | [`tray`]: `editorReplace` panel over `work-tray.tab` services, `emptyArrowDown` intent, `Shift+↓` (`unipi.tray.open`), hint line below the editor, service `core.open-tray` |
//! | settings | `decision-model` | the shared namespace ([`unipi_kit::jev`]); round 5 (D1) removed `hints` |
//!
//! Round 5 deleted the presentational half of unipi core (D1): the Unicrab hint catalog, the
//! hint widget above the editor, the Unicrab start header, the `/hint` command and browser,
//! the hint store and the `hints` settings namespace. The desktop has its own welcome and
//! glance surfaces. The work tray and the `unipi-migrate` skill stay.
//!
//! Everything else in unipi core (bus, arbiter, command echo, runners, child agents, …) is a host
//! feature or lives in `unipi-kit`.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; this init registers synchronously"
)]

pub mod tray;

use std::time::Duration;

use oino_extension::prelude::*;
use oino_extension::sync::SessionMap;
use serde_json::Value;
use unipi_kit::bus::Delivery;
use unipi_kit::services::{OpenTray, fanout as fan, point};
use unipi_kit::tray::{
    Action, ActionReply, TabRequest, TrayTab, decode_tabs, item_count, sorted_visible, split_action,
};
use unipi_kit::{fanout, jev, topics};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.core";

/// Topics whose change re-reads the tray tabs.
const TRAY_TOPICS: &[&str] = &[
    topics::WORK_TRAY_CHANGED,
    topics::BG_TASKS,
    topics::SUBAGENTS_LIST,
    topics::FUSION_STATUS,
];

#[derive(Default)]
struct SessionState {
    panel: Option<String>,
    active: Option<String>,
    tabs: Vec<TrayTab>,
}

/// The extension.
#[derive(Default)]
pub struct Core {
    sessions: SessionMap<SessionState>,
}

impl Core {
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
        let open = self.sessions.with(cx.session_id(), |s| s.panel.is_some());
        let tabs = Self::fetch_tabs(open, None).await;
        let (panel, active) = self.sessions.with(cx.session_id(), |s| {
            s.tabs.clone_from(&tabs);
            (s.panel.clone(), s.active.clone())
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
        let (panel, remembered) = self
            .sessions
            .with(cx.session_id(), |s| (s.panel.clone(), s.active.clone()));
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
            *s = SessionState {
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
                    s.panel = None;
                    s.tabs.clone()
                });
                let _ = ui::fire(&tray::strip_request(&tabs, false));
            }
            tray::TAB_ACTION => {
                let tab = value
                    .as_ref()
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.sessions.with(&sid, |s| s.active = tab);
                self.refresh_tray(cx).await;
            }
            other => {
                let Some((tab, action)) = split_action(other) else {
                    return;
                };
                let owner = self.sessions.with(&sid, |s| {
                    s.tabs
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
                    let panel = self.sessions.with(&sid, |s| s.panel.take());
                    if let Some(id) = panel {
                        ui::panel_close(&id);
                    }
                    let tabs = self.sessions.with(&sid, |s| s.tabs.clone());
                    let _ = ui::fire(&tray::strip_request(&tabs, false));
                } else if reply.refresh {
                    self.refresh_tray(cx).await;
                }
            }
        }
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
        let tray_id = self.sessions.with(cx.session_id(), |s| s.panel.clone());
        if Some(&panel) == tray_id.as_ref() || panel == tray::PANEL {
            self.tray_action(cx, &action, value).await;
        }
        Ok(())
    }

    async fn on_editor(&self, ev: &Event, cx: &Ctx) -> Option<Value> {
        if ev.get("intent").and_then(Value::as_str) != Some("emptyArrowDown") {
            return None;
        }
        let (open, count) = self.sessions.with(cx.session_id(), |s| {
            (s.panel.is_some(), item_count(&s.tabs))
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
        self.sessions.remove(cx.session_id());
        self.refresh_tray(cx).await;
    }

    async fn on_bus(&self, ev: &Event, cx: &Ctx) {
        let Some(d) = Delivery::from_event(ev) else {
            return;
        };
        if TRAY_TOPICS.contains(&d.topic.as_str()) {
            self.refresh_tray(cx).await;
        }
    }
}

impl Extension for Core {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let decision = jev::namespace(EXTENSION_ID);
        cx.settings_namespace(&decision.to_json());

        cx.on(EventKind::SessionStart, |ext, _ev, cx| async move {
            ext.on_session_start(&cx).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |ext, _ev, cx| async move {
            ext.sessions.remove(cx.session_id());
            Ok(())
        });
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
        bus::subscribe("unipi:*");

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
