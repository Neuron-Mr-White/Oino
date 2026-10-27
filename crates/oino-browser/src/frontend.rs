//! Viewer side (browser-control.md §8.5, §9): screencast views, user input forwarding with the
//! take-over rule, user tabs, handoff answers, clipboard bridge, cursor probe, site
//! permissions and profiles.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_browser_protocol::view::modifiers;
use oino_browser_protocol::{
    BrowserInput, BrowserViewEvent, ControlledBy, FrontendRequest, KeyEventType, MouseButton,
    MouseEventType, TabLifecycle, TabOwner, Viewport,
};
use oino_extension_api::browser::ViewStream;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::page;
use crate::policy;
use crate::profile::{ProfileRt, push_frame};
use crate::service::BrowserService;
use crate::tab::{Tab, View, lock};

fn owned_by(tab: &Tab, session: Option<&str>) -> bool {
    lock(&tab.state).owner.session() == session
}

/// Attaches a viewer (starts the screencast on the first one).
pub(crate) fn attach_view(
    svc: &BrowserService,
    session: Option<&str>,
    tab_id: &str,
    viewport: Option<Viewport>,
) -> Result<ViewStream, String> {
    let (rt, tab) = svc
        .find_tab(tab_id)
        .ok_or_else(|| format!("tab {tab_id} is not open"))?;
    if !owned_by(&tab, session) {
        return Err(format!("tab {tab_id} belongs to another conversation"));
    }
    let id = svc.hub().view_id();
    let (tx, rx) = mpsc::channel(8);
    lock(&tab.views).push(View {
        id,
        tx,
        unacked: 0,
        last_seq: 0,
    });
    lock(&svc.inner.views).insert(id, (rt.name.clone(), tab.id.clone()));
    if let Some(v) = viewport {
        let mut s = lock(&tab.state);
        if s.fit {
            s.viewport = v;
        }
    }
    let start = !std::mem::replace(&mut lock(&tab.state).screencast, true);
    let rt2 = Arc::clone(&rt);
    let tab2 = Arc::clone(&tab);
    tokio::spawn(async move {
        if start {
            start_screencast(&rt2, &tab2).await;
        }
        first_frame(&rt2, &tab2).await;
        rt2.page_changed(&tab2);
    });
    let events = Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|e| (e, rx))
    }));
    Ok(ViewStream {
        view_id: id,
        events,
    })
}

async fn start_screencast(rt: &ProfileRt, tab: &Tab) {
    let vp = lock(&tab.state).viewport;
    let w = (f64::from(vp.width) * vp.scale).min(2560.0);
    let h = (f64::from(vp.height) * vp.scale).min(2560.0);
    let _ = rt
        .cdp
        .call(
            Some(&tab.session),
            "Page.startScreencast",
            json!({"format": "jpeg", "quality": 70, "maxWidth": page::px(w), "maxHeight": page::px(h), "everyNthFrame": 1}),
        )
        .await;
}

/// A still frame so a new viewer is never blank (screencast emits only on damage).
async fn first_frame(rt: &ProfileRt, tab: &Tab) {
    use base64::Engine as _;
    let Ok(r) = rt
        .cdp
        .call(
            Some(&tab.session),
            "Page.captureScreenshot",
            json!({"format": "jpeg", "quality": 70}),
        )
        .await
    else {
        return;
    };
    let Some(data) = r["data"]
        .as_str()
        .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
    else {
        return;
    };
    let vp = lock(&tab.state).viewport;
    push_frame(
        tab,
        &oino_browser_protocol::FrameData(Arc::new(data)),
        f64::from(vp.width),
        f64::from(vp.height),
    );
}

fn detach(svc: &BrowserService, view: u64) {
    let Some((_, tab_id)) = lock(&svc.inner.views).remove(&view) else {
        return;
    };
    let Some((rt, tab)) = svc.find_tab(&tab_id) else {
        return;
    };
    let empty = {
        let mut views = lock(&tab.views);
        views.retain(|v| v.id != view && !v.tx.is_closed());
        views.is_empty()
    };
    if empty {
        lock(&tab.state).screencast = false;
        let cdp = Arc::clone(&rt.cdp);
        let session = tab.session.clone();
        tokio::spawn(async move {
            let _ = cdp
                .call(Some(&session), "Page.stopScreencast", json!({}))
                .await;
        });
    }
}

/// Sets take-over state and tells viewers.
pub(crate) fn set_takeover(rt: &ProfileRt, tab: &Tab, on: bool) {
    let (session, driving) = {
        let mut s = lock(&tab.state);
        s.controlled_by = if on {
            ControlledBy::User
        } else {
            ControlledBy::Agent
        };
        if on {
            s.last_user_input = Some(Instant::now());
        }
        (s.owner.session().map(str::to_owned), s.driving)
    };
    let e = BrowserViewEvent::Agent {
        tab: tab.id.clone(),
        driving,
        taken_over: on,
    };
    tab.broadcast_view(&e);
    rt.hub.emit(session.as_deref(), e);
    tab.changed.notify_waiters();
}

/// Ends a pending handoff (Done / Cancel / timeout).
pub(crate) fn end_handoff(rt: &ProfileRt, tab: &Tab, done: bool) {
    let (pending, session) = {
        let mut s = lock(&tab.state);
        let pending = s.handoff.take();
        if s.lifecycle == TabLifecycle::Handoff {
            s.lifecycle = if s.owner.is_agent() {
                TabLifecycle::Active
            } else {
                TabLifecycle::User
            };
        }
        (pending, s.owner.session().map(str::to_owned))
    };
    let message = pending.map(|(m, tx)| {
        let _ = tx.send(done);
        m
    });
    set_takeover(rt, tab, false);
    let e = BrowserViewEvent::Handoff {
        tab: tab.id.clone(),
        message: message.unwrap_or_default(),
        pending: false,
    };
    tab.broadcast_view(&e);
    rt.hub.emit(session.as_deref(), e);
}

/// Handles one frontend request.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per `_oino/browser_*` request"
)]
pub(crate) async fn handle(
    svc: &BrowserService,
    session: Option<&str>,
    workspace: &str,
    req: FrontendRequest,
) -> Result<Value, String> {
    let tab_of = |id: &str| -> Result<(Arc<ProfileRt>, Arc<Tab>), String> {
        let (rt, tab) = svc
            .find_tab(id)
            .ok_or_else(|| format!("tab {id} is not open"))?;
        if owned_by(&tab, session) {
            Ok((rt, tab))
        } else {
            Err(format!("tab {id} belongs to another conversation"))
        }
    };
    match req {
        FrontendRequest::TabOpen { url, reveal } => {
            policy::check_user_url(&url, &svc.settings().blocked_origins).map_err(|b| b.0)?;
            if let Some(s) = session {
                svc.hub().session_tx(s, workspace);
            }
            let scope =
                oino_browser_protocol::BrowserScope::session(session.unwrap_or("-"), workspace);
            let rt = svc
                .ensure(&svc.profile_of(&scope))
                .await
                .map_err(|r| r.text)?;
            let tab = svc
                .open_tab(
                    &rt,
                    TabOwner::User {
                        session: session.map(str::to_owned),
                    },
                    &url,
                )
                .await
                .map_err(|e| e.message)?;
            if reveal {
                rt.hub.emit(
                    session,
                    BrowserViewEvent::Reveal {
                        tab: tab.id.clone(),
                        reason: "user".into(),
                    },
                );
            }
            serde_json::to_value(tab.info()).map_err(|e| e.to_string())
        }
        FrontendRequest::TabClose { tab } => {
            let (rt, t) = tab_of(&tab)?;
            rt.close_tab(&t, "user").await;
            Ok(Value::Null)
        }
        FrontendRequest::Input { tab, input } => {
            let (rt, t) = tab_of(&tab)?;
            input_event(svc, &rt, &t, input).await?;
            Ok(Value::Null)
        }
        FrontendRequest::Takeover { tab, on } => {
            let (rt, t) = tab_of(&tab)?;
            if !on && lock(&t.state).handoff.is_some() {
                end_handoff(&rt, &t, true);
            } else {
                set_takeover(&rt, &t, on);
            }
            let s = lock(&t.state);
            Ok(json!({"driving": s.driving, "takenOver": s.controlled_by == ControlledBy::User}))
        }
        FrontendRequest::HandoffDone { tab, done } => {
            let (rt, t) = tab_of(&tab)?;
            end_handoff(&rt, &t, done);
            Ok(Value::Null)
        }
        FrontendRequest::DialogAnswer { tab, accept, text } => {
            let (rt, t) = tab_of(&tab)?;
            let mut p = json!({"accept": accept});
            if let Some(text) = text {
                p["promptText"] = json!(text);
            }
            rt.cdp
                .call(Some(&t.session), "Page.handleJavaScriptDialog", p)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        FrontendRequest::CopySelection { tab } => {
            let (rt, t) = tab_of(&tab)?;
            let v = page::evaluate(
                &rt.cdp,
                &t,
                "() => { let a = document.activeElement; while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; if (a && (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA') && a.selectionStart !== a.selectionEnd) return a.value.slice(a.selectionStart, a.selectionEnd); return String(getSelection()); }",
                None,
                false,
            )
            .await
            .map_err(|e| e.message)?;
            Ok(v)
        }
        FrontendRequest::Cursor { tab, x, y } => {
            let (rt, t) = tab_of(&tab)?;
            let v = page::evaluate(
                &rt.cdp,
                &t,
                &format!("() => {{ const e = document.elementFromPoint({x}, {y}); return e ? getComputedStyle(e).cursor : 'default'; }}"),
                None,
                false,
            )
            .await
            .map_err(|e| e.message)?;
            Ok(v)
        }
        FrontendRequest::ViewAck { view_id, seq } => {
            if let Some((_, tab)) = lock(&svc.inner.views).get(&view_id).cloned()
                && let Some((_, t)) = svc.find_tab(&tab)
            {
                for v in lock(&t.views).iter_mut().filter(|v| v.id == view_id) {
                    let behind = v.last_seq.saturating_sub(seq);
                    v.unacked = u32::try_from(behind).unwrap_or(u32::MAX).min(v.unacked);
                }
            }
            Ok(Value::Null)
        }
        FrontendRequest::ViewDetach { view_id } => {
            detach(svc, view_id);
            Ok(Value::Null)
        }
        FrontendRequest::SitePermissions { origin, set } => {
            let path = svc.hub().paths.site_permissions();
            let grants = match set {
                Some((perm, setting)) => {
                    let g = crate::permissions::set(
                        &path,
                        &origin,
                        &perm,
                        (setting != "ask").then_some(setting.as_str()),
                    )
                    .map_err(|e| e.to_string())?;
                    let live: Vec<Arc<ProfileRt>> =
                        lock(&svc.inner.live).values().cloned().collect();
                    let one: crate::permissions::Grants = [(
                        origin.clone(),
                        [(
                            perm.clone(),
                            if setting == "ask" {
                                "prompt".into()
                            } else {
                                setting.clone()
                            },
                        )]
                        .into_iter()
                        .collect(),
                    )]
                    .into_iter()
                    .collect();
                    for p in live {
                        p.apply_permissions(&one).await;
                    }
                    g
                }
                None => crate::permissions::load(&path),
            };
            Ok(json!({"perms": grants.get(&origin).cloned().unwrap_or_default()}))
        }
        FrontendRequest::Profiles { op, name } => {
            let paths = svc.hub().paths.clone();
            match (op.as_str(), name) {
                ("create", Some(n)) => crate::profiles::create(&paths, &n)?,
                ("remove", Some(n)) => {
                    if lock(&svc.inner.live).contains_key(&n) {
                        return Err(format!("profile {n} is running; stop it first"));
                    }
                    crate::profiles::remove(&paths, &n)?;
                }
                ("use", Some(n)) => {
                    if let Some(s) = session
                        && let Some(rt) = lock(&svc.hub().sessions).get_mut(s)
                    {
                        rt.profile = Some(n);
                    }
                }
                ("list", _) => {}
                _ => return Err(format!("unknown profiles action {op}")),
            }
            serde_json::to_value(svc.profiles()).map_err(|e| e.to_string())
        }
    }
}

fn button(b: MouseButton) -> (&'static str, u32) {
    match b {
        MouseButton::Left => ("left", 1),
        MouseButton::Right => ("right", 2),
        MouseButton::Middle => ("middle", 4),
    }
}

/// Marks user input; enters take-over when the agent is busy on the tab (§7.4).
fn user_input(rt: &ProfileRt, tab: &Tab, counts: bool) {
    if !counts {
        return;
    }
    let take = {
        let mut s = lock(&tab.state);
        s.last_user_input = Some(Instant::now());
        s.last_activity = Instant::now();
        s.controlled_by == ControlledBy::Agent
            && s.owner.is_agent()
            && (s.driving
                || s.last_agent_cmd
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(3)))
    };
    if take {
        set_takeover(rt, tab, true);
    }
}

#[allow(clippy::too_many_lines, reason = "one arm per input kind")]
async fn input_event(
    svc: &BrowserService,
    rt: &ProfileRt,
    tab: &Tab,
    input: BrowserInput,
) -> Result<(), String> {
    let s = Some(tab.session.as_str());
    let call = |m: &'static str, p: Value| {
        let cdp = Arc::clone(&rt.cdp);
        async move {
            cdp.call(s, m, p)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    };
    match input {
        BrowserInput::Mouse {
            event,
            x,
            y,
            button: b,
            click_count,
            modifiers,
        } => {
            user_input(rt, tab, event == MouseEventType::Down);
            let (name, bits) = button(b);
            let kind = match event {
                MouseEventType::Down => "mousePressed",
                MouseEventType::Up => "mouseReleased",
                MouseEventType::Move => "mouseMoved",
            };
            let mut p = json!({"type": kind, "x": x, "y": y, "modifiers": modifiers,
                               "clickCount": click_count.max(u32::from(event != MouseEventType::Move))});
            if event != MouseEventType::Move {
                p["button"] = json!(name);
                p["buttons"] = json!(if event == MouseEventType::Down {
                    bits
                } else {
                    0
                });
            }
            lock(&tab.state).pointer = (x, y);
            call("Input.dispatchMouseEvent", p).await
        }
        BrowserInput::Wheel {
            x,
            y,
            dx,
            dy,
            modifiers,
        } => {
            user_input(rt, tab, true);
            call(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy, "modifiers": modifiers}),
            )
            .await
        }
        BrowserInput::Key {
            event,
            key,
            text,
            modifiers: m,
        } => {
            user_input(rt, tab, event == KeyEventType::Down);
            let def = crate::keys::key_def(&key);
            let (code, kc) = def
                .as_ref()
                .map_or((String::new(), 0), |d| (d.code.clone(), d.key_code));
            let mut p =
                json!({"key": key, "code": code, "windowsVirtualKeyCode": kc, "modifiers": m});
            match event {
                KeyEventType::Down => {
                    let printable = text.as_ref().filter(|t| {
                        !t.is_empty()
                            && m & (modifiers::CTRL | modifiers::META | modifiers::ALT) == 0
                    });
                    if let Some(t) = printable {
                        p["type"] = json!("keyDown");
                        p["text"] = json!(t);
                        p["unmodifiedText"] = json!(t);
                    } else {
                        p["type"] = json!("rawKeyDown");
                        if let Some(t) = def
                            .and_then(|d| d.text)
                            .filter(|_| m & (modifiers::CTRL | modifiers::META) == 0)
                        {
                            p["type"] = json!("keyDown");
                            p["text"] = json!(t);
                        }
                    }
                }
                KeyEventType::Up => p["type"] = json!("keyUp"),
            }
            call("Input.dispatchKeyEvent", p).await
        }
        BrowserInput::Text { text } | BrowserInput::ClipboardPaste { text } => {
            user_input(rt, tab, true);
            call("Input.insertText", json!({"text": text})).await
        }
        BrowserInput::Ime {
            text,
            selection_start,
            selection_end,
        } => {
            user_input(rt, tab, true);
            call(
                "Input.imeSetComposition",
                json!({"text": text, "selectionStart": selection_start, "selectionEnd": selection_end}),
            )
            .await
        }
        BrowserInput::Goto { url } => {
            user_input(rt, tab, true);
            policy::check_user_url(&url, &svc.settings().blocked_origins).map_err(|b| b.0)?;
            call("Page.navigate", json!({"url": url})).await
        }
        BrowserInput::History { action } => {
            user_input(rt, tab, true);
            match action.as_str() {
                "reload" => call("Page.reload", json!({})).await,
                "stop" => call("Page.stopLoading", json!({})).await,
                "back" | "forward" => {
                    let h = rt
                        .cdp
                        .call(s, "Page.getNavigationHistory", json!({}))
                        .await
                        .map_err(|e| e.to_string())?;
                    let idx = h["currentIndex"].as_i64().unwrap_or(0);
                    let want = if action == "back" { idx - 1 } else { idx + 1 };
                    let Some(entry) = h["entries"]
                        .as_array()
                        .and_then(|e| usize::try_from(want).ok().and_then(|w| e.get(w)))
                    else {
                        return Ok(());
                    };
                    call(
                        "Page.navigateToHistoryEntry",
                        json!({"entryId": entry["id"]}),
                    )
                    .await
                }
                other => Err(format!("unknown history action {other}")),
            }
        }
        BrowserInput::Resize { viewport } => {
            let busy = {
                let mut st = lock(&tab.state);
                st.fit = true;
                st.viewport = viewport;
                st.driving
            };
            if busy {
                return Ok(());
            }
            call(
                "Emulation.setDeviceMetricsOverride",
                json!({"width": viewport.width, "height": viewport.height,
                       "deviceScaleFactor": viewport.scale, "mobile": viewport.mobile}),
            )
            .await?;
            if lock(&tab.state).screencast {
                let _ = call("Page.stopScreencast", json!({})).await;
                start_screencast(rt, tab).await;
            }
            Ok(())
        }
    }
}
