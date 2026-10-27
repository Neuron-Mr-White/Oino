//! The [`BrowserHub`] global: the process-wide browser service (installed by the desktop
//! binary) and per-pane reveal watchers (browser-control.md §9.2: agent `reveal` events open
//! the session's Browser tab).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui_kit::{App, Context, Global, Task, WeakEntity};
use oino_extension_api::browser::BrowserHost;
use oino_extension_api::browser::protocol::BrowserViewEvent;

use crate::views::session_pane::{SessionPane, SideTab};

/// The browser service and watchers.
pub struct BrowserHub {
    /// The service.
    pub host: Arc<dyn BrowserHost>,
    watchers: RefCell<HashMap<String, Task<()>>>,
    reveals: RefCell<HashMap<String, String>>,
}

impl Global for BrowserHub {}

impl BrowserHub {
    /// Installs the service (desktop shell).
    pub fn install(cx: &mut App, host: Arc<dyn BrowserHost>) {
        cx.set_global(Self {
            host,
            watchers: RefCell::new(HashMap::new()),
            reveals: RefCell::new(HashMap::new()),
        });
    }

    /// The service, when installed.
    #[must_use]
    pub fn host(cx: &App) -> Option<Arc<dyn BrowserHost>> {
        cx.try_global::<Self>().map(|h| Arc::clone(&h.host))
    }

    /// The tab the agent last revealed in `session` (taken once).
    #[must_use]
    pub fn take_reveal(cx: &App, session: &str) -> Option<String> {
        cx.try_global::<Self>()
            .and_then(|h| h.reveals.borrow_mut().remove(session))
    }
}

/// Starts (once per session) a watcher that opens the pane's Browser tab when the agent opens
/// or selects a tab or asks for a handoff. Called from the pane every frame; cheap after the
/// first call.
pub fn watch(pane: &SessionPane, cx: &mut Context<SessionPane>) {
    let session = pane.model.id.clone();
    if session.is_empty() {
        return;
    }
    let host = match cx.try_global::<BrowserHub>() {
        Some(hub) if !hub.watchers.borrow().contains_key(&session) => Arc::clone(&hub.host),
        _ => return,
    };
    let mut events = host.subscribe(&session);
    let weak: WeakEntity<SessionPane> = cx.entity().downgrade();
    let key = session.clone();
    let task = cx.spawn(async move |_, cx| {
        while let Some(e) = events.next().await {
            let BrowserViewEvent::Reveal { tab, reason } = e else {
                continue;
            };
            if !matches!(reason.as_str(), "agent_opened" | "handoff") {
                continue;
            }
            let alive = weak.update(cx, |p, cx| {
                if let Some(hub) = cx.try_global::<BrowserHub>() {
                    hub.reveals.borrow_mut().insert(key.clone(), tab.clone());
                }
                if p.browser.is_none() || p.side.current() != Some(&SideTab::Browser) {
                    p.open_side(SideTab::Browser, cx);
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }
        cx.update(|cx| {
            if let Some(hub) = cx.try_global::<BrowserHub>() {
                hub.watchers.borrow_mut().remove(&key);
            }
        });
    });
    cx.global::<BrowserHub>()
        .watchers
        .borrow_mut()
        .insert(session, task);
}
