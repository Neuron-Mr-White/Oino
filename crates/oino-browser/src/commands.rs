//! Agent command execution (browser-control.md §5): scope → tab, gate, approvals, per-op
//! actions, event collection and result formatting.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_browser_protocol::command::{
    DownloadsAction, KeepAs, LogKind, ProfileAction, RefStateKind, TabsAction,
};
use oino_browser_protocol::{
    BrowserCommand, BrowserDetails, BrowserErrorCode, BrowserImage, BrowserResult, BrowserScope,
    BrowserToolDetails, BrowserViewEvent, ControlledBy, HistoryAction, Modifier, MouseButton,
    SideEffect, TabLifecycle, TabOwner, ToolEvent, WaitLoad, redact_url,
};
use oino_extension_api::browser::{Approval, BrowserRequest};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::{Approvals, EvaluatePolicy, Humanize};
use crate::page::{self, ActionError, ActionResult, Human};
use crate::policy;
use crate::profile::ProfileRt;
use crate::service::BrowserService;
use crate::tab::{Tab, lock};

/// What an op produced.
#[derive(Debug, Default)]
struct Outcome {
    text: String,
    data: Option<Value>,
    image: Option<BrowserImage>,
    snapshot_id: Option<String>,
    want_snapshot: bool,
    tab: Option<Arc<Tab>>,
}

impl Outcome {
    fn text(t: impl Into<String>) -> Self {
        Self {
            text: t.into(),
            ..Self::default()
        }
    }
}

fn err(code: BrowserErrorCode, msg: impl Into<String>) -> ActionError {
    ActionError::new(code, msg)
}

fn into_result(e: ActionError) -> BrowserResult {
    BrowserResult::error(e.code, e.message, e.side)
}

/// Entry point of [`oino_extension_api::browser::BrowserHost::execute`].
pub(crate) async fn execute(
    svc: &BrowserService,
    scope: BrowserScope,
    request: BrowserRequest,
    cancel: CancellationToken,
) -> BrowserResult {
    let settings = svc.settings();
    if !settings.enabled {
        return BrowserResult::unavailable(
            "disabled",
            "browser control is off (Settings → Browser, `browser.enabled`)",
        );
    }
    svc.hub().session_tx(&scope.session_id, &scope.workspace);
    let BrowserRequest { command, approval } = request;
    match &command {
        BrowserCommand::Status => {
            let st = svc.status_for_scope(&scope);
            return BrowserResult {
                text: st.summary(),
                details: BrowserToolDetails {
                    data: serde_json::to_value(&st).ok(),
                    ..BrowserToolDetails::default()
                },
                ..BrowserResult::default()
            };
        }
        BrowserCommand::Profile { action, name } => {
            return profile_op(svc, &scope, *action, name.as_deref(), approval);
        }
        BrowserCommand::TurnEnded => return BrowserResult::ok("ok"),
        BrowserCommand::SessionClosed { close_tabs } => {
            session_closed(svc, &scope.session_id, *close_tabs).await;
            return BrowserResult::ok("ok");
        }
        BrowserCommand::CloseAgentTabs => {
            let n = close_agent_tabs(svc, &scope.session_id).await;
            return BrowserResult::ok(format!("Closed {n} agent tab(s)"));
        }
        BrowserCommand::Downloads { action, timeout_ms } => {
            return downloads(svc, &scope, *action, *timeout_ms, &cancel).await;
        }
        _ => {}
    }
    let profile = svc.profile_of(&scope);
    let rt = match svc.ensure(&profile).await {
        Ok(rt) => rt,
        Err(r) => return r,
    };
    let human = match settings.humanize {
        Humanize::Careful => Human::Careful,
        Humanize::Default => Human::On,
        Humanize::Auto if rt.engine == "cloak" => Human::On,
        Humanize::Auto | Humanize::Off => Human::Off,
    };
    let cx = Cx {
        svc,
        rt: &rt,
        scope: &scope,
        approval,
        human,
        cancel: &cancel,
    };
    let result = run(&cx, &command).await;
    let mut out = match result {
        Ok(o) => success(&cx, &command, o).await,
        Err(e) => into_result(e),
    };
    if out.details.browser.is_none() {
        out.details.browser = Some(base_details(&cx, None));
    }
    out
}

impl BrowserService {
    pub(crate) fn status_for_scope(
        &self,
        scope: &BrowserScope,
    ) -> oino_browser_protocol::BrowserStatus {
        oino_extension_api::browser::BrowserHost::status(self, Some(scope))
    }
}

struct Cx<'a> {
    svc: &'a BrowserService,
    rt: &'a Arc<ProfileRt>,
    scope: &'a BrowserScope,
    approval: Approval,
    human: Human,
    cancel: &'a CancellationToken,
}

impl Cx<'_> {
    fn session(&self) -> &str {
        &self.scope.session_id
    }

    /// Tabs the agent of this session may drive.
    fn agent_tabs(&self) -> Vec<Arc<Tab>> {
        self.rt
            .all_tabs()
            .into_iter()
            .filter(|t| usable(t, self.session()))
            .collect()
    }

    fn user_tabs(&self) -> Vec<Arc<Tab>> {
        self.rt
            .all_tabs()
            .into_iter()
            .filter(|t| {
                let s = lock(&t.state);
                matches!(&s.owner, TabOwner::User { session: Some(x) } if x == self.session())
                    && s.lifecycle == TabLifecycle::User
            })
            .collect()
    }

    fn resolve_tab(&self, explicit: Option<&str>) -> ActionResult<Arc<Tab>> {
        if let Some(id) = explicit {
            let Some((rt, tab)) = self.svc.find_tab(id) else {
                return Err(err(
                    BrowserErrorCode::TabNotFound,
                    format!("tab {id} is not open; use browser_tabs list"),
                ));
            };
            if !Arc::ptr_eq(&rt, self.rt) && !usable(&tab, self.session()) {
                return Err(err(
                    BrowserErrorCode::TabNotFound,
                    format!("tab {id} is not in this conversation"),
                ));
            }
            if usable(&tab, self.session()) {
                return Ok(tab);
            }
            if self.user_tabs().iter().any(|t| t.id == id) {
                return Err(err(
                    BrowserErrorCode::TabNotFound,
                    format!(
                        "{id} is the user's tab; claim it first with browser_tabs {{action:\"claim\", tab:\"{id}\"}}"
                    ),
                ));
            }
            return Err(err(
                BrowserErrorCode::TabNotFound,
                format!("tab {id} is not in this conversation; use browser_tabs list"),
            ));
        }
        if let Some(cur) = self.svc.hub().current(self.session())
            && let Some(t) = self.rt.tab(&cur).filter(|t| usable(t, self.session()))
        {
            return Ok(t);
        }
        let agent = self.agent_tabs();
        if agent.len() == 1 {
            return Ok(Arc::clone(&agent[0]));
        }
        if agent.is_empty() {
            let visible: Vec<Arc<Tab>> = self
                .user_tabs()
                .into_iter()
                .filter(|t| !lock(&t.views).is_empty() && claimable(t))
                .collect();
            if visible.len() == 1 {
                return Ok(Arc::clone(&visible[0]));
            }
            return Err(err(
                BrowserErrorCode::TabRequired,
                "no browser tab is open in this conversation; open one with browser_navigate {url}",
            ));
        }
        let ids: Vec<String> = agent.iter().map(|t| t.id.clone()).collect();
        Err(err(
            BrowserErrorCode::TabRequired,
            format!("several tabs are open ({}); pass tab", ids.join(", ")),
        ))
    }

    fn reveal(&self, tab: &Tab, reason: &str) {
        self.svc
            .hub()
            .set_current(self.session(), Some(tab.id.clone()));
        lock(&tab.state).last_selected = Some(Instant::now());
        self.svc.hub().emit(
            Some(self.session()),
            BrowserViewEvent::Reveal {
                tab: tab.id.clone(),
                reason: reason.into(),
            },
        );
    }

    fn needs_approval(&self, origin: &str) -> bool {
        match self.approval {
            Approval::Once | Approval::Site => {
                if self.approval == Approval::Site {
                    lock(&self.svc.inner.site_allow)
                        .insert((self.session().to_owned(), origin.to_owned()));
                }
                false
            }
            Approval::None => !lock(&self.svc.inner.site_allow)
                .contains(&(self.session().to_owned(), origin.to_owned())),
        }
    }
}

fn usable(t: &Tab, session: &str) -> bool {
    let s = lock(&t.state);
    match &s.owner {
        TabOwner::Agent { session: x } => x == session,
        TabOwner::User { session: Some(x) } => x == session && s.lifecycle != TabLifecycle::User,
        TabOwner::User { session: None } => false,
    }
}

fn claimable(t: &Tab) -> bool {
    let s = lock(&t.state);
    !s.url.is_empty() && s.url != "about:blank"
}

fn approval_required(message: String) -> ActionError {
    err(BrowserErrorCode::ApprovalRequired, message)
}

fn mod_bits(mods: &[Modifier]) -> u8 {
    use oino_browser_protocol::view::modifiers as m;
    mods.iter().fold(0, |b, x| {
        b | match x {
            Modifier::Alt => m::ALT,
            Modifier::Control => m::CTRL,
            Modifier::Shift => m::SHIFT,
            Modifier::ControlOrMeta if !cfg!(target_os = "macos") => m::CTRL,
            Modifier::Meta | Modifier::ControlOrMeta => m::META,
        }
    })
}

async fn run(cx: &Cx<'_>, cmd: &BrowserCommand) -> ActionResult<Outcome> {
    match cmd {
        BrowserCommand::Navigate {
            tab,
            url,
            history,
            new_tab,
            snapshot,
        } => {
            navigate(
                cx,
                tab.as_deref(),
                url.as_deref(),
                *history,
                *new_tab,
                snapshot.unwrap_or(true),
            )
            .await
        }
        BrowserCommand::Tabs {
            action,
            tab,
            url,
            keep_as,
        } => tabs(cx, *action, tab.as_deref(), url.as_deref(), *keep_as).await,
        BrowserCommand::Handoff {
            tab,
            message,
            wait,
            timeout_s,
        } => {
            let t = cx.resolve_tab(tab.as_deref())?;
            handoff(
                cx,
                &t,
                message,
                wait.unwrap_or(true),
                timeout_s.unwrap_or(600).min(1800),
            )
            .await
        }
        BrowserCommand::Release { tab } => {
            let tabs: Vec<Arc<Tab>> = match tab {
                Some(id) => vec![cx.resolve_tab(Some(id))?],
                None => cx.agent_tabs(),
            };
            let mut n = 0;
            for t in tabs {
                let mut s = lock(&t.state);
                if matches!(s.owner, TabOwner::User { .. }) {
                    s.lifecycle = TabLifecycle::User;
                    n += 1;
                }
            }
            Ok(Outcome::text(format!("Released {n} tab(s) to the user")))
        }
        BrowserCommand::Reveal { tab } => {
            let t = cx.resolve_tab(Some(tab))?;
            cx.reveal(&t, "agent_selected");
            Ok(Outcome {
                text: format!("Revealed {}", t.id),
                tab: Some(t),
                ..Outcome::default()
            })
        }
        BrowserCommand::TurnShot { turn_id } => turn_shot(cx, turn_id).await,
        other => {
            let t = cx.resolve_tab(other.tab())?;
            on_tab(cx, &t, other).await
        }
    }
}

/// Runs a tab command under the tab's command lock with gate, dialog and event collection.
async fn on_tab(cx: &Cx<'_>, tab: &Arc<Tab>, cmd: &BrowserCommand) -> ActionResult<Outcome> {
    {
        let s = lock(&tab.state);
        if !policy::gate(s.controlled_by, cmd) {
            return Err(err(
                BrowserErrorCode::TakenOver,
                format!(
                    "The user has control of tab {}. Wait, or ask them (browser_handoff).",
                    tab.id
                ),
            ));
        }
        if let Some(d) = &s.dialog
            && cmd.is_driving()
            && !matches!(cmd, BrowserCommand::HandleDialog { .. })
        {
            return Err(err(
                BrowserErrorCode::DialogOpen,
                format!(
                    "a {} dialog is open (\"{}\"); answer it with browser_handle_dialog first",
                    d.kind, d.message
                ),
            ));
        }
    }
    let _guard = tokio::select! {
        g = tab.cmd.lock() => g,
        () = cx.cancel.cancelled() => return Err(err(BrowserErrorCode::Cancelled, "cancelled before it started")),
    };
    let driving = cmd.is_driving();
    if driving {
        set_driving(cx, tab, true);
    }
    lock(&tab.state).collecting = Some(Vec::new());
    let r = tokio::select! {
        r = op(cx, tab, cmd) => r,
        () = cx.cancel.cancelled() => Err(err(BrowserErrorCode::Cancelled, "cancelled").uncertain_if(driving)),
    };
    if r.is_ok() && cmd.is_action() {
        settle(tab).await;
    }
    if driving {
        set_driving(cx, tab, false);
    }
    r.map(|mut o| {
        o.tab.get_or_insert_with(|| Arc::clone(tab));
        o
    })
}

impl ActionError {
    fn uncertain_if(self, yes: bool) -> Self {
        if yes { self.uncertain() } else { self }
    }
}

fn set_driving(cx: &Cx<'_>, tab: &Tab, on: bool) {
    let taken = {
        let mut s = lock(&tab.state);
        if on && s.collecting.is_none() {
            s.collecting = Some(Vec::new());
        }
        s.driving = on;
        s.last_agent_cmd = Some(Instant::now());
        s.last_activity = Instant::now();
        s.controlled_by == ControlledBy::User
    };
    let e = BrowserViewEvent::Agent {
        tab: tab.id.clone(),
        driving: on,
        taken_over: taken,
    };
    tab.broadcast_view(&e);
    cx.svc.hub().emit(Some(cx.session()), e);
}

/// Waits ≤ 1.5 s for the page to go quiet after an action (§7.3).
async fn settle(tab: &Tab) {
    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(120)).await;
    loop {
        let (busy, quiet_for, loading) = {
            let s = lock(&tab.state);
            (!s.inflight.is_empty(), s.last_network.elapsed(), s.loading)
        };
        if (!busy && !loading) || start.elapsed() > Duration::from_millis(1500) {
            return;
        }
        if !loading && quiet_for > Duration::from_millis(500) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[allow(clippy::too_many_lines, reason = "one dispatch arm per tool")]
async fn op(cx: &Cx<'_>, tab: &Arc<Tab>, cmd: &BrowserCommand) -> ActionResult<Outcome> {
    let cdp = &cx.rt.cdp;
    match cmd {
        BrowserCommand::Snapshot {
            scope,
            max_chars,
            frames,
            ..
        } => {
            let max = max_chars.unwrap_or(24_000).clamp(500, 60_000) as usize;
            let (text, sid) =
                page::take_snapshot(cdp, tab, scope.as_deref(), max, frames.unwrap_or(true))
                    .await?;
            Ok(Outcome {
                text,
                snapshot_id: Some(sid),
                ..Outcome::default()
            })
        }
        BrowserCommand::Click {
            target,
            x,
            y,
            button,
            double,
            modifiers,
            sensitive,
            snapshot,
            ..
        } => {
            let (point, label) = point_for(cx, tab, target.as_deref(), *x, *y, true).await?;
            check_sensitive(cx, tab, *sensitive, &label, "click")?;
            page::click_at(
                cdp,
                tab,
                point,
                button.unwrap_or(MouseButton::Left),
                if *double { 2 } else { 1 },
                mod_bits(modifiers),
                cx.human,
            )
            .await?;
            Ok(Outcome {
                text: format!("Clicked {label}"),
                want_snapshot: *snapshot,
                ..Outcome::default()
            })
        }
        BrowserCommand::Hover { target, x, y, .. } => {
            let (point, label) = point_for(cx, tab, target.as_deref(), *x, *y, false).await?;
            page::move_to(cdp, tab, point, cx.human, 0).await?;
            Ok(Outcome::text(format!("Hovering {label}")))
        }
        BrowserCommand::Type {
            target,
            text,
            clear,
            submit,
            sensitive,
            snapshot,
            ..
        } => {
            let label = type_into(cx, tab, target.as_deref(), clear.unwrap_or(true)).await?;
            if *submit {
                check_sensitive(cx, tab, *sensitive, &label, "submit")?;
            } else if *sensitive {
                check_sensitive(cx, tab, true, &label, "type into")?;
            }
            page::insert_text(cdp, tab, text, cx.human).await?;
            if *submit && let Some(chord) = crate::keys::parse_chord("Enter", false) {
                page::press(cdp, tab, &chord).await?;
            }
            Ok(Outcome {
                text: format!(
                    "Typed {} chars into {label}{}",
                    text.chars().count(),
                    if *submit { " and pressed Enter" } else { "" }
                ),
                want_snapshot: *snapshot,
                ..Outcome::default()
            })
        }
        BrowserCommand::PressKey {
            key,
            target,
            sensitive,
            snapshot,
            ..
        } => {
            let chord = crate::keys::parse_chord(key, cfg!(target_os = "macos")).ok_or_else(|| {
                err(BrowserErrorCode::ExecutionError, format!("unknown key {key:?}; use Playwright names like Enter, Control+a, ArrowDown"))
            })?;
            let mut label = "the focused element".to_owned();
            if let Some(r) = target {
                let e = page::resolve_ref(tab, r)?;
                let _ = cdp
                    .call(
                        Some(&e.session),
                        "DOM.focus",
                        json!({"backendNodeId": e.backend}),
                    )
                    .await;
                label = format!("{} \"{}\" [{r}]", e.role, e.name);
            }
            if chord.key.key == "Enter" {
                check_sensitive(cx, tab, *sensitive, &label, "press Enter in")?;
            }
            page::press(cdp, tab, &chord).await?;
            Ok(Outcome {
                text: format!("Pressed {key}"),
                want_snapshot: *snapshot,
                ..Outcome::default()
            })
        }
        BrowserCommand::Scroll {
            target,
            dx,
            dy,
            x,
            y,
            ..
        } => {
            if let Some(r) = target {
                let e = page::resolve_ref(tab, r)?;
                page::object_of(cdp, r, &e).await?;
                cdp.call(
                    Some(&e.session),
                    "DOM.scrollIntoViewIfNeeded",
                    json!({"backendNodeId": e.backend}),
                )
                .await?;
            } else {
                let l = page::layout(cdp, tab).await?;
                let at = (x.unwrap_or(l.width / 2.0), y.unwrap_or(l.height / 2.0));
                let (dx, dy) = (dx.unwrap_or(0.0), dy.unwrap_or(0.0));
                let notches = if cx.human == Human::Off {
                    1
                } else {
                    notches_for(dx, dy)
                };
                for _ in 0..notches {
                    cdp.call(
                        Some(&tab.session),
                        "Input.dispatchMouseEvent",
                        json!({"type": "mouseWheel", "x": at.0, "y": at.1,
                               "deltaX": dx / f64::from(notches), "deltaY": dy / f64::from(notches)}),
                    )
                    .await?;
                    if notches > 1 {
                        tokio::time::sleep(Duration::from_millis(30)).await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            let l = page::layout(cdp, tab).await?;
            Ok(Outcome::text(format!(
                "Scrolled to {:.0},{:.0} (page {:.0}×{:.0})",
                l.scroll_x, l.scroll_y, l.content_width, l.content_height
            )))
        }
        BrowserCommand::SelectOption { target, values, .. } => {
            let e = page::resolve_ref(tab, target)?;
            let obj = page::object_of(cdp, target, &e).await?;
            let r = page::call_on(cdp, &e.session, &obj, SELECT_JS, &[json!(values)]).await?;
            if let Some(m) = r.get("error").and_then(Value::as_str) {
                return Err(err(
                    BrowserErrorCode::ExecutionError,
                    format!("{target}: {m}"),
                ));
            }
            let selected: Vec<String> = r["selected"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if selected.is_empty() {
                return Err(err(
                    BrowserErrorCode::ExecutionError,
                    format!(
                        "none of {values:?} matched an option of {target}; take a snapshot to see the options"
                    ),
                ));
            }
            Ok(Outcome::text(format!(
                "Selected {} in {} \"{}\" [{target}]",
                selected.join(", "),
                e.role,
                e.name
            )))
        }
        BrowserCommand::Drag {
            from_ref,
            to_ref,
            from,
            to,
            ..
        } => {
            let a = point_for(
                cx,
                tab,
                from_ref.as_deref(),
                from.map(|p| p.x),
                from.map(|p| p.y),
                true,
            )
            .await?;
            let b = point_for(
                cx,
                tab,
                to_ref.as_deref(),
                to.map(|p| p.x),
                to.map(|p| p.y),
                false,
            )
            .await?;
            page::drag(cdp, tab, a.0, b.0).await?;
            Ok(Outcome::text(format!("Dragged {} to {}", a.1, b.1)))
        }
        BrowserCommand::FileUpload { target, paths, .. } => {
            upload(cx, tab, target.as_deref(), paths).await
        }
        BrowserCommand::HandleDialog {
            accept,
            prompt_text,
            ..
        } => {
            let d = lock(&tab.state).dialog.clone().ok_or_else(|| {
                err(
                    BrowserErrorCode::ExecutionError,
                    "no dialog is open on this tab",
                )
            })?;
            let mut p = json!({"accept": accept});
            if let Some(t) = prompt_text {
                p["promptText"] = json!(t);
            }
            cdp.call(Some(&tab.session), "Page.handleJavaScriptDialog", p)
                .await?;
            lock(&tab.state).dialog = None;
            Ok(Outcome::text(format!(
                "{} the {} dialog \"{}\"",
                if *accept { "Accepted" } else { "Dismissed" },
                d.kind,
                d.message
            )))
        }
        BrowserCommand::TakeScreenshot {
            target, full_page, ..
        } => screenshot(cx, tab, target.as_deref(), *full_page).await,
        BrowserCommand::Evaluate {
            function,
            target,
            await_promise,
            ..
        } => {
            match cx.svc.settings().evaluate {
                EvaluatePolicy::Deny => {
                    return Err(err(
                        BrowserErrorCode::PermissionDenied,
                        "browser_evaluate is disabled (browser.evaluate = deny)",
                    ));
                }
                EvaluatePolicy::Ask if cx.approval == Approval::None => {
                    return Err(approval_required(format!(
                        "Agent wants to run JavaScript on {}",
                        host_of(tab)
                    )));
                }
                _ => {}
            }
            let entry = match target {
                Some(r) => Some((r.as_str(), page::resolve_ref(tab, r)?)),
                None => None,
            };
            let v = page::evaluate(
                cdp,
                tab,
                function,
                entry.as_ref().map(|(r, e)| (*r, e)),
                await_promise.unwrap_or(true),
            )
            .await?;
            let mut text = serde_json::to_string_pretty(&v).unwrap_or_default();
            if text.len() > 20_000 {
                let mut cut = 20_000;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                text.push_str("\n… (truncated)");
            }
            Ok(Outcome {
                text: format!("Result:\n{text}"),
                ..Outcome::default()
            })
        }
        BrowserCommand::WaitFor { .. } => wait_for(cx, tab, cmd).await,
        BrowserCommand::Logs {
            kind,
            since_seq,
            filter,
            limit,
            ..
        } => Ok(logs(
            tab,
            *kind,
            *since_seq,
            filter.as_deref(),
            limit.unwrap_or(50).min(200) as usize,
        )),
        _ => Err(err(BrowserErrorCode::ExecutionError, "unsupported command")),
    }
}

const SELECT_JS: &str = "function(values){if(this.tagName!=='SELECT')return{error:'not a <select> element'};const want=new Set(values);let n=0;for(const o of this.options){const hit=want.has(o.value)||want.has(o.label)||want.has(o.textContent.trim());if(hit&&(this.multiple||n===0)){o.selected=true;n++;}else if(!this.multiple||!hit){o.selected=false;}}if(n===0)return{selected:[]};this.dispatchEvent(new Event('input',{bubbles:true}));this.dispatchEvent(new Event('change',{bubbles:true}));return{selected:[...this.selectedOptions].map(o=>o.label||o.value)}}";

fn host_of(tab: &Tab) -> String {
    let url = lock(&tab.state).url.clone();
    policy::hostname(&url).unwrap_or(url)
}

/// Humanized wheel notches (one per ~100 px, 1–40).
fn notches_for(dx: f64, dy: f64) -> u32 {
    let n = ((dy.abs() + dx.abs()) / 100.0).ceil().clamp(1.0, 40.0);
    (1..=40u32).find(|i| f64::from(*i) >= n).unwrap_or(40)
}

/// Approval for sensitive actions (§10.2).
fn check_sensitive(
    cx: &Cx<'_>,
    tab: &Tab,
    sensitive: bool,
    label: &str,
    verb: &str,
) -> ActionResult<()> {
    let url = lock(&tab.state).url.clone();
    let origin = policy::origin(&url).unwrap_or_default();
    let approvals = cx.svc.settings().approvals;
    let heuristic = approvals == Approvals::Default
        && !policy::is_loopback(&url)
        && policy::looks_sensitive(label);
    if (sensitive && approvals == Approvals::Default || heuristic) && cx.needs_approval(&origin) {
        return Err(approval_required(format!(
            "Agent wants to {verb} {label} on {}",
            host_of(tab)
        )));
    }
    Ok(())
}

/// Ref or coordinates → (point, label).
async fn point_for(
    cx: &Cx<'_>,
    tab: &Tab,
    target: Option<&str>,
    x: Option<f64>,
    y: Option<f64>,
    need_enabled: bool,
) -> ActionResult<((f64, f64), String)> {
    match (target, x, y) {
        (Some(r), ..) => {
            let entry = page::resolve_ref(tab, r)?;
            let point = page::actionable_point(&cx.rt.cdp, tab, r, &entry, need_enabled).await?;
            Ok((point, format!("{} \"{}\" [{r}]", entry.role, entry.name)))
        }
        (None, Some(x), Some(y)) => Ok(((x, y), format!("at {x:.0},{y:.0}"))),
        _ => Err(err(
            BrowserErrorCode::ExecutionError,
            "pass ref (from a snapshot) or x and y",
        )),
    }
}

const FOCUS_CHECK: &str = "function(){let a=document.activeElement;while(a&&a.shadowRoot&&a.shadowRoot.activeElement)a=a.shadowRoot.activeElement;const ed=e=>e&&(e.isContentEditable||((e.tagName==='INPUT'||e.tagName==='TEXTAREA')&&!e.readOnly&&!e.disabled));return (a===this||this.contains(a))&&ed(a)}";
const CLEAR_JS: &str = "function(){let a=document.activeElement;while(a&&a.shadowRoot&&a.shadowRoot.activeElement)a=a.shadowRoot.activeElement;const e=(a&&(this===a||this.contains(a)))?a:this;if(e.select){e.select();return true}const r=document.createRange();r.selectNodeContents(e);const s=getSelection();s.removeAllRanges();s.addRange(r);return true}";
const ACTIVE_EDITABLE: &str = "() => { let a = document.activeElement; while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement; return !!a && (a.isContentEditable || ((a.tagName === 'INPUT' || a.tagName === 'TEXTAREA') && !a.readOnly && !a.disabled)); }";

/// Focuses the target (click if needed), optionally clears; returns its label.
async fn type_into(
    cx: &Cx<'_>,
    tab: &Tab,
    target: Option<&str>,
    clear: bool,
) -> ActionResult<String> {
    let cdp = &cx.rt.cdp;
    let Some(r) = target else {
        let ok = page::evaluate(cdp, tab, ACTIVE_EDITABLE, None, false).await?;
        if ok != json!(true) {
            return Err(err(
                BrowserErrorCode::NotEditable,
                "no editable element has focus; pass ref of a textbox",
            ));
        }
        if clear {
            let chord = crate::keys::parse_chord(
                if cfg!(target_os = "macos") {
                    "Meta+a"
                } else {
                    "Control+a"
                },
                false,
            );
            if let Some(c) = chord {
                page::press(cdp, tab, &c).await?;
                if let Some(bs) = crate::keys::parse_chord("Backspace", false) {
                    page::press(cdp, tab, &bs).await?;
                }
            }
        }
        return Ok("the focused element".into());
    };
    let e = page::resolve_ref(tab, r)?;
    let obj = page::object_of(cdp, r, &e).await?;
    let label = format!("{} \"{}\" [{r}]", e.role, e.name);
    let _ = cdp
        .call(
            Some(&e.session),
            "DOM.focus",
            json!({"backendNodeId": e.backend}),
        )
        .await;
    if page::call_on(cdp, &e.session, &obj, FOCUS_CHECK, &[]).await? != json!(true) {
        let p = page::actionable_point(cdp, tab, r, &e, true).await?;
        page::click_at(cdp, tab, p, MouseButton::Left, 1, 0, cx.human).await?;
        if page::call_on(cdp, &e.session, &obj, FOCUS_CHECK, &[]).await? != json!(true) {
            return Err(err(
                BrowserErrorCode::NotEditable,
                format!(
                    "{label} is not editable; pick a textbox, textarea or contenteditable element"
                ),
            ));
        }
    }
    if clear {
        page::call_on(cdp, &e.session, &obj, CLEAR_JS, &[]).await?;
        if let Some(bs) = crate::keys::parse_chord("Backspace", false) {
            page::press(cdp, tab, &bs).await?;
        }
    }
    Ok(label)
}

#[allow(
    clippy::too_many_lines,
    reason = "one step list per operation, clearer kept together"
)]
async fn navigate(
    cx: &Cx<'_>,
    tab: Option<&str>,
    url: Option<&str>,
    history: Option<HistoryAction>,
    new_tab: bool,
    want_snapshot: bool,
) -> ActionResult<Outcome> {
    let cdp = &cx.rt.cdp;
    let (target, note) = match (url, history) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(err(
                BrowserErrorCode::ExecutionError,
                "pass exactly one of url or history",
            ));
        }
        (None, Some(h)) => {
            let t = cx.resolve_tab(tab)?;
            check_gate(&t)?;
            let r = {
                let _g = t.cmd.lock().await;
                set_driving(cx, &t, true);
                let r = history_nav(cdp, &t, h).await;
                set_driving(cx, &t, false);
                r
            };
            r?;
            (t, String::new())
        }
        (Some(raw), None) => {
            let url = normalise(raw);
            let cwd = std::path::PathBuf::from(&cx.scope.workspace);
            let s = cx.svc.settings();
            policy::check_agent_url(&url, &cwd, &s.allowed_origins, &s.blocked_origins).map_err(
                |b| {
                    err(
                        BrowserErrorCode::NavigationBlocked,
                        format!("{url}: {}", b.0),
                    )
                },
            )?;
            let existing = match tab {
                Some(id) => Some(cx.resolve_tab(Some(id))?),
                None if new_tab => None,
                None => reuse_candidate(cx, &url),
            };
            if let Some(t) = existing {
                check_gate(&t)?;
                let r = {
                    let _g = t.cmd.lock().await;
                    set_driving(cx, &t, true);
                    let r = goto(cx, &t, &url).await;
                    set_driving(cx, &t, false);
                    r
                };
                r?;
                cx.reveal(&t, "agent_selected");
                let note = if tab.is_none() {
                    format!(" (reused tab {})", t.id)
                } else {
                    String::new()
                };
                (t, note)
            } else {
                let t = cx
                    .svc
                    .open_tab(
                        cx.rt,
                        TabOwner::Agent {
                            session: cx.session().to_owned(),
                        },
                        "about:blank",
                    )
                    .await?;
                cx.reveal(&t, "agent_opened");
                let r = {
                    let _g = t.cmd.lock().await;
                    set_driving(cx, &t, true);
                    let r = goto(cx, &t, &url).await;
                    set_driving(cx, &t, false);
                    r
                };
                r?;
                (t, " (new tab)".to_owned())
            }
        }
    };
    page::refresh_info(cdp, &target).await;
    let loading = page::probe(cdp, &target, "document.readyState")
        .await
        .is_ok_and(|v| v == json!("loading"));
    let (u, title) = {
        let s = lock(&target.state);
        (s.url.clone(), s.title.clone())
    };
    Ok(Outcome {
        text: format!(
            "Opened {} · \"{}\" · {u}{note}{}",
            target.id,
            if title.is_empty() {
                "(untitled)"
            } else {
                &title
            },
            if loading { " · still loading" } else { "" }
        ),
        want_snapshot,
        tab: Some(target),
        ..Outcome::default()
    })
}

fn check_gate(t: &Tab) -> ActionResult<()> {
    let s = lock(&t.state);
    if s.controlled_by == ControlledBy::User {
        return Err(err(
            BrowserErrorCode::TakenOver,
            format!(
                "The user has control of tab {}. Wait, or ask them (browser_handoff).",
                t.id
            ),
        ));
    }
    if let Some(d) = &s.dialog
        && d.kind != "beforeunload"
    {
        return Err(err(
            BrowserErrorCode::DialogOpen,
            format!(
                "a {} dialog is open (\"{}\"); answer it with browser_handle_dialog first",
                d.kind, d.message
            ),
        ));
    }
    Ok(())
}

fn normalise(raw: &str) -> String {
    let s = raw.trim();
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        return s.to_owned();
    }
    let host = s.split(['/', '?', '#']).next().unwrap_or_default();
    if host.starts_with("localhost") || host.starts_with("127.") {
        format!("http://{s}")
    } else {
        format!("https://{s}")
    }
}

/// Same-hostname reuse (§5.2 #1): exact URL > origin+path > hostname; current first, then recent.
fn reuse_candidate(cx: &Cx<'_>, url: &str) -> Option<Arc<Tab>> {
    let host = policy::hostname(url)?;
    let current = cx.svc.hub().current(cx.session());
    let strip = |u: &str| {
        u.split(['?', '#'])
            .next()
            .unwrap_or(u)
            .trim_end_matches('/')
            .to_owned()
    };
    cx.agent_tabs()
        .into_iter()
        .filter_map(|t| {
            let (tu, act) = {
                let s = lock(&t.state);
                (s.url.clone(), s.last_activity)
            };
            if policy::hostname(&tu)? != host {
                return None;
            }
            let rank = if tu == url {
                3
            } else if strip(&tu) == strip(url) {
                2
            } else {
                1
            };
            let is_current = current.as_deref() == Some(t.id.as_str());
            Some(((rank, is_current, act), t))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, t)| t)
}

async fn goto(cx: &Cx<'_>, tab: &Tab, url: &str) -> ActionResult<()> {
    let cdp = &cx.rt.cdp;
    lock(&tab.state).loading = true;
    let r = cdp
        .call_timeout(
            Some(&tab.session),
            "Page.navigate",
            json!({"url": url}),
            Duration::from_secs(30),
        )
        .await
        .map_err(|e| ActionError::from(e).uncertain())?;
    if let Some(e) = r.get("errorText").and_then(Value::as_str)
        && !e.is_empty()
    {
        lock(&tab.state).loading = false;
        return Err(err(
            BrowserErrorCode::ExecutionError,
            format!("could not open {url}: {e}; check the address or that the server runs"),
        )
        .uncertain());
    }
    wait_ready(cx, tab, Duration::from_secs(10)).await;
    Ok(())
}

async fn wait_ready(cx: &Cx<'_>, tab: &Tab, limit: Duration) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Ok(v) = page::probe(&cx.rt.cdp, tab, "document.readyState").await
            && v.as_str().is_some_and(|s| s != "loading")
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn history_nav(cdp: &crate::cdp::Cdp, tab: &Tab, h: HistoryAction) -> ActionResult<()> {
    let s = Some(tab.session.as_str());
    match h {
        HistoryAction::Reload => {
            cdp.call(s, "Page.reload", json!({})).await?;
        }
        HistoryAction::Back | HistoryAction::Forward => {
            let hist = cdp.call(s, "Page.getNavigationHistory", json!({})).await?;
            let idx = hist["currentIndex"].as_i64().unwrap_or(0);
            let want = if h == HistoryAction::Back {
                idx - 1
            } else {
                idx + 1
            };
            let entry = hist["entries"]
                .as_array()
                .and_then(|e| usize::try_from(want).ok().and_then(|w| e.get(w)))
                .ok_or_else(|| {
                    err(
                        BrowserErrorCode::ExecutionError,
                        "no history entry in that direction",
                    )
                })?;
            cdp.call(
                s,
                "Page.navigateToHistoryEntry",
                json!({"entryId": entry["id"]}),
            )
            .await?;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "one step list per operation, clearer kept together"
)]
async fn tabs(
    cx: &Cx<'_>,
    action: TabsAction,
    tab: Option<&str>,
    url: Option<&str>,
    keep_as: Option<KeepAs>,
) -> ActionResult<Outcome> {
    match action {
        TabsAction::List => {
            let current = cx.svc.hub().current(cx.session());
            let mut text = String::new();
            let mut list = Vec::new();
            for t in cx.agent_tabs() {
                let i = t.info();
                let _ = writeln!(
                    text,
                    "{} · agent · {} · \"{}\" · {}{}",
                    i.id,
                    i.lifecycle.as_str(),
                    i.title,
                    i.url,
                    if current.as_deref() == Some(i.id.as_str()) {
                        " (current)"
                    } else {
                        ""
                    }
                );
                list.push(i);
            }
            for t in cx.user_tabs().into_iter().filter(|t| claimable(t)) {
                let i = t.info();
                let _ = writeln!(
                    text,
                    "{} · user (claimable) · \"{}\" · {}",
                    i.id, i.title, i.url
                );
                list.push(i);
            }
            if text.is_empty() {
                text = "No tabs in this conversation. Open one with browser_navigate.".into();
            }
            Ok(Outcome {
                text: text.trim_end().to_owned(),
                data: serde_json::to_value(&list).ok(),
                ..Outcome::default()
            })
        }
        TabsAction::Select => {
            let t = cx.resolve_tab(Some(
                tab.ok_or_else(|| err(BrowserErrorCode::TabRequired, "pass tab"))?,
            ))?;
            cx.reveal(&t, "agent_selected");
            Ok(Outcome {
                text: format!("Selected {}", t.id),
                tab: Some(t),
                ..Outcome::default()
            })
        }
        TabsAction::New => {
            let url = url.map_or_else(|| "about:blank".to_owned(), normalise);
            let s = cx.svc.settings();
            policy::check_agent_url(
                &url,
                std::path::Path::new(&cx.scope.workspace),
                &s.allowed_origins,
                &s.blocked_origins,
            )
            .map_err(|b| err(BrowserErrorCode::NavigationBlocked, b.0))?;
            let t = cx
                .svc
                .open_tab(
                    cx.rt,
                    TabOwner::Agent {
                        session: cx.session().to_owned(),
                    },
                    "about:blank",
                )
                .await?;
            cx.reveal(&t, "agent_opened");
            if url != "about:blank" {
                goto(cx, &t, &url).await?;
            }
            Ok(Outcome {
                text: format!("Opened {} · {url}", t.id),
                tab: Some(t),
                ..Outcome::default()
            })
        }
        TabsAction::Close => {
            let id = tab.ok_or_else(|| err(BrowserErrorCode::TabRequired, "pass tab"))?;
            let t = cx.resolve_tab(Some(id))?;
            let user = matches!(lock(&t.state).owner, TabOwner::User { .. });
            if user && cx.approval == Approval::None {
                return Err(approval_required(format!(
                    "Agent wants to close your tab {id} ({})",
                    host_of(&t)
                )));
            }
            cx.rt.close_tab(&t, "agent").await;
            Ok(Outcome::text(format!("Closed {id}")))
        }
        TabsAction::Claim => {
            let id = tab.ok_or_else(|| err(BrowserErrorCode::TabRequired, "pass tab"))?;
            let t = cx
                .user_tabs()
                .into_iter()
                .find(|t| t.id == id)
                .ok_or_else(|| {
                    err(
                        BrowserErrorCode::TabNotFound,
                        format!("{id} is not a claimable user tab of this conversation"),
                    )
                })?;
            if !claimable(&t) {
                return Err(err(
                    BrowserErrorCode::TabNotFound,
                    format!("{id} is empty; open a page with browser_navigate instead"),
                ));
            }
            lock(&t.state).lifecycle = TabLifecycle::Active;
            cx.svc.hub().set_current(cx.session(), Some(t.id.clone()));
            Ok(Outcome {
                text: format!("Claimed {id}; the user still sees it and can take over"),
                tab: Some(t),
                ..Outcome::default()
            })
        }
        TabsAction::Keep => {
            let t = cx.resolve_tab(tab)?;
            let lc = match keep_as.unwrap_or(KeepAs::Deliverable) {
                KeepAs::Deliverable => TabLifecycle::Deliverable,
                KeepAs::Handoff => TabLifecycle::Handoff,
            };
            lock(&t.state).lifecycle = lc;
            Ok(Outcome {
                text: format!("Kept {} as {} for the user", t.id, lc.as_str()),
                tab: Some(t),
                ..Outcome::default()
            })
        }
    }
}

async fn handoff(
    cx: &Cx<'_>,
    tab: &Arc<Tab>,
    message: &str,
    wait: bool,
    timeout_s: u64,
) -> ActionResult<Outcome> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut s = lock(&tab.state);
        s.controlled_by = ControlledBy::User;
        s.lifecycle = TabLifecycle::Handoff;
        s.handoff = Some((message.to_owned(), tx));
    }
    cx.reveal(tab, "handoff");
    let e = BrowserViewEvent::Handoff {
        tab: tab.id.clone(),
        message: message.to_owned(),
        pending: true,
    };
    tab.broadcast_view(&e);
    cx.svc.hub().emit(Some(cx.session()), e);
    set_driving(cx, tab, false);
    if !wait {
        return Ok(Outcome {
            text: format!("Asked the user to act in {}: {message}", tab.id),
            tab: Some(Arc::clone(tab)),
            ..Outcome::default()
        });
    }
    let outcome = tokio::select! {
        r = rx => match r { Ok(true) => "done", _ => "cancelled" },
        () = tokio::time::sleep(Duration::from_secs(timeout_s)) => "timeout",
        () = cx.cancel.cancelled() => "cancelled",
    };
    crate::frontend::end_handoff(cx.rt, tab, outcome == "done");
    let (url, title) = {
        let s = lock(&tab.state);
        (s.url.clone(), s.title.clone())
    };
    Ok(Outcome {
        text: format!("Handoff {outcome} · {} · \"{title}\" · {url}", tab.id),
        data: Some(json!({"outcome": outcome, "url": redact_url(&url), "title": title})),
        tab: Some(Arc::clone(tab)),
        ..Outcome::default()
    })
}

async fn upload(
    cx: &Cx<'_>,
    tab: &Arc<Tab>,
    target: Option<&str>,
    paths: &[String],
) -> ActionResult<Outcome> {
    let cdp = &cx.rt.cdp;
    let cwd = std::path::Path::new(&cx.scope.workspace);
    let mut files = Vec::new();
    for p in paths {
        let path = if std::path::Path::new(p).is_absolute() {
            std::path::PathBuf::from(p)
        } else {
            cwd.join(p)
        };
        if !path.is_file() {
            return Err(err(
                BrowserErrorCode::ExecutionError,
                format!("{} does not exist", path.display()),
            ));
        }
        files.push(path.display().to_string());
    }
    if cx.approval == Approval::None {
        return Err(approval_required(format!(
            "Agent wants to upload {} to {}",
            files.join(", "),
            host_of(tab)
        )));
    }
    let r = target.ok_or_else(|| {
        err(
            BrowserErrorCode::ExecutionError,
            "pass ref of the file input or its button",
        )
    })?;
    let e = page::resolve_ref(tab, r)?;
    let obj = page::object_of(cdp, r, &e).await?;
    let is_input = page::call_on(
        cdp,
        &e.session,
        &obj,
        "function(){return this.tagName==='INPUT'&&this.type==='file'}",
        &[],
    )
    .await?;
    if is_input == json!(true) {
        cdp.call(
            Some(&e.session),
            "DOM.setFileInputFiles",
            json!({"files": files, "backendNodeId": e.backend}),
        )
        .await?;
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        lock(&tab.state).file_chooser = Some(tx);
        cdp.call(
            Some(&tab.session),
            "Page.setInterceptFileChooserDialog",
            json!({"enabled": true}),
        )
        .await?;
        let p = page::actionable_point(cdp, tab, r, &e, true).await?;
        page::click_at(cdp, tab, p, MouseButton::Left, 1, 0, cx.human).await?;
        let node = tokio::time::timeout(page::ROUTINE, rx).await;
        let _ = cdp
            .call(
                Some(&tab.session),
                "Page.setInterceptFileChooserDialog",
                json!({"enabled": false}),
            )
            .await;
        let Ok(Ok(node)) = node else {
            return Err(err(
                BrowserErrorCode::Timeout,
                format!("clicking {r} opened no file chooser; pass the file input's ref"),
            )
            .uncertain());
        };
        cdp.call(
            Some(&tab.session),
            "DOM.setFileInputFiles",
            json!({"files": files, "backendNodeId": node}),
        )
        .await?;
    }
    Ok(Outcome::text(format!(
        "Uploaded {} file(s) via {} \"{}\" [{r}]",
        files.len(),
        e.role,
        e.name
    )))
}

async fn screenshot(
    cx: &Cx<'_>,
    tab: &Arc<Tab>,
    target: Option<&str>,
    full: bool,
) -> ActionResult<Outcome> {
    let cdp = &cx.rt.cdp;
    let clip = match target {
        Some(r) => {
            let e = page::resolve_ref(tab, r)?;
            Some(page::element_box(cdp, r, &e).await?)
        }
        None => None,
    };
    let png = page::screenshot_png(cdp, tab, clip, full).await?;
    let dir = cx.svc.hub().paths.shots(cx.session());
    let _ = std::fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{}-{stamp}.png", tab.id));
    std::fs::write(&path, &png)
        .map_err(|e| err(BrowserErrorCode::ExecutionError, e.to_string()))?;
    let c = crate::screenshot::compress(&png).ok_or_else(|| {
        err(
            BrowserErrorCode::ExecutionError,
            "could not encode the screenshot",
        )
    })?;
    Ok(Outcome {
        text: format!(
            "Screenshot of {} ({}×{}) saved to {}",
            tab.id,
            c.width,
            c.height,
            path.display()
        ),
        image: Some(BrowserImage {
            mime_type: c.mime.to_owned(),
            data: c.data,
            path: Some(path.display().to_string()),
        }),
        ..Outcome::default()
    })
}

async fn turn_shot(cx: &Cx<'_>, turn_id: &str) -> ActionResult<Outcome> {
    let tab = cx.resolve_tab(None)?;
    let png = page::screenshot_png(&cx.rt.cdp, &tab, None, false).await?;
    let c = crate::screenshot::compress(&png)
        .ok_or_else(|| err(BrowserErrorCode::ExecutionError, "encode failed"))?;
    let dir = cx.svc.hub().paths.shots(cx.session());
    let _ = std::fs::create_dir_all(&dir);
    let ext = if c.mime == "image/png" { "png" } else { "jpg" };
    let path = dir.join(format!("{}.{ext}", crate::hub::sanitize(turn_id)));
    std::fs::write(&path, &c.data)
        .map_err(|e| err(BrowserErrorCode::ExecutionError, e.to_string()))?;
    let (url, title) = {
        let s = lock(&tab.state);
        (s.url.clone(), s.title.clone())
    };
    Ok(Outcome {
        text: format!("Turn screenshot saved to {}", path.display()),
        data: Some(
            json!({"path": path.display().to_string(), "tab": tab.id, "url": redact_url(&url),
                          "title": title, "width": c.width, "height": c.height}),
        ),
        tab: Some(tab),
        ..Outcome::default()
    })
}

fn glob_match(pattern: &str, url: &str) -> bool {
    globset::Glob::new(pattern).is_ok_and(|g| g.compile_matcher().is_match(url))
        || url.contains(pattern)
}

#[allow(
    clippy::too_many_lines,
    reason = "one step list per operation, clearer kept together"
)]
async fn wait_for(cx: &Cx<'_>, tab: &Arc<Tab>, cmd: &BrowserCommand) -> ActionResult<Outcome> {
    let BrowserCommand::WaitFor {
        text,
        text_gone,
        ref_state,
        url,
        load,
        time_ms,
        timeout_ms,
        ..
    } = cmd
    else {
        return Err(err(BrowserErrorCode::ExecutionError, "bad wait"));
    };
    if let Some(ms) = time_ms {
        tokio::time::sleep(Duration::from_millis((*ms).min(30_000))).await;
        return Ok(Outcome::text(format!("Waited {ms} ms")));
    }
    let limit = Duration::from_millis(timeout_ms.unwrap_or(5000).min(30_000));
    let start = Instant::now();
    let cdp = &cx.rt.cdp;
    let mut what;
    loop {
        let done = if let Some(t) = text {
            what = format!("text \"{t}\"");
            page::probe(
                cdp,
                tab,
                &format!(
                    "!!document.body && document.body.innerText.includes({})",
                    json!(t)
                ),
            )
            .await?
                == json!(true)
        } else if let Some(t) = text_gone {
            what = format!("text \"{t}\" to disappear");
            page::probe(
                cdp,
                tab,
                &format!(
                    "!document.body || !document.body.innerText.includes({})",
                    json!(t)
                ),
            )
            .await?
                == json!(true)
        } else if let Some(u) = url {
            what = format!("URL {u}");
            glob_match(u, &lock(&tab.state).url)
        } else if let Some(l) = load {
            what = format!("{l:?} load").to_lowercase();
            match l {
                WaitLoad::Domcontentloaded => {
                    page::probe(cdp, tab, "document.readyState !== 'loading'").await? == json!(true)
                }
                WaitLoad::Load => {
                    page::probe(cdp, tab, "document.readyState === 'complete'").await?
                        == json!(true)
                }
                WaitLoad::Networkidle => {
                    let s = lock(&tab.state);
                    s.inflight.is_empty()
                        && s.last_network.elapsed() > Duration::from_millis(500)
                        && !s.loading
                }
            }
        } else if let Some(rs) = ref_state {
            what = format!("{} to be {:?}", rs.target, rs.state).to_lowercase();
            let e = page::resolve_ref(tab, &rs.target)?;
            let obj = page::object_of(cdp, &rs.target, &e).await;
            match (rs.state, obj) {
                (RefStateKind::Detached | RefStateKind::Hidden, Err(_)) => true,
                (RefStateKind::Detached, Ok(o)) => {
                    page::call_on(
                        cdp,
                        &e.session,
                        &o,
                        "function(){return !this.isConnected}",
                        &[],
                    )
                    .await?
                        == json!(true)
                }
                (RefStateKind::Visible | RefStateKind::Enabled, Err(e)) => return Err(e),
                (state, Ok(o)) => {
                    let f = match state {
                        RefStateKind::Visible => {
                            "function(){const r=this.getBoundingClientRect();return this.isConnected&&r.width>0&&r.height>0&&getComputedStyle(this).visibility!=='hidden'}"
                        }
                        RefStateKind::Hidden => {
                            "function(){const r=this.getBoundingClientRect();return !this.isConnected||r.width===0||r.height===0||getComputedStyle(this).visibility==='hidden'}"
                        }
                        _ => {
                            "function(){return !this.disabled&&!this.closest('[aria-disabled=\"true\"]')}"
                        }
                    };
                    page::call_on(cdp, &e.session, &o, f, &[]).await? == json!(true)
                }
            }
        } else {
            return Err(err(
                BrowserErrorCode::ExecutionError,
                "pass one of text, text_gone, ref_state, url, load or time_ms",
            ));
        };
        if done {
            return Ok(Outcome::text(format!(
                "Done waiting for {what} ({} ms)",
                start.elapsed().as_millis()
            )));
        }
        if start.elapsed() > limit {
            return Err(err(
                BrowserErrorCode::Timeout,
                format!(
                    "{what} did not happen within {} ms; take a snapshot to see the page",
                    limit.as_millis()
                ),
            ));
        }
        tokio::select! {
            () = tab.changed.notified() => {},
            () = tokio::time::sleep(Duration::from_millis(150)) => {},
        }
    }
}

fn logs(
    tab: &Tab,
    kind: LogKind,
    since: Option<u64>,
    filter: Option<&str>,
    limit: usize,
) -> Outcome {
    let re = filter
        .and_then(|f| f.strip_prefix('/').and_then(|r| r.strip_suffix('/')))
        .and_then(|r| regex::Regex::new(r).ok());
    let keep = |t: &str| match (&re, filter) {
        (Some(re), _) => re.is_match(t),
        (None, Some(f)) => t.contains(f),
        (None, None) => true,
    };
    let s = lock(&tab.state);
    let since = since.unwrap_or(0);
    let lines: Vec<String> = match kind {
        LogKind::Console | LogKind::Errors => {
            let ring = if kind == LogKind::Console {
                &s.console
            } else {
                &s.errors
            };
            ring.iter()
                .filter(|l| l.seq > since && keep(&l.text))
                .map(|l| format!("#{} {}", l.seq, l.text))
                .collect()
        }
        LogKind::Network => s
            .network
            .iter()
            .filter(|n| n.seq > since)
            .map(|n| {
                format!(
                    "#{} {} {} → {} {} {}{}",
                    n.seq,
                    n.method,
                    n.url,
                    n.status.map_or_else(|| "…".to_owned(), |c| c.to_string()),
                    n.kind,
                    n.size.map_or_else(String::new, |b| format!("{b} B ")),
                    n.ms.map_or_else(String::new, |m| format!("{m:.0} ms"))
                ) + &n
                    .failed
                    .as_ref()
                    .map_or_else(String::new, |f| format!(" ({f})"))
            })
            .filter(|l| keep(l))
            .collect(),
    };
    let skip = lines.len().saturating_sub(limit);
    let shown = &lines[skip..];
    let text = if shown.is_empty() {
        format!("No {kind:?} entries.").to_lowercase()
    } else {
        format!(
            "{}{}",
            if skip > 0 {
                format!("({skip} older entries omitted)\n")
            } else {
                String::new()
            },
            shown.join("\n")
        )
    };
    Outcome::text(text)
}

async fn downloads(
    svc: &BrowserService,
    scope: &BrowserScope,
    action: DownloadsAction,
    timeout_ms: Option<u64>,
    cancel: &CancellationToken,
) -> BrowserResult {
    let list = || -> Vec<crate::profile::Download> {
        lock(&svc.inner.live)
            .values()
            .flat_map(|p| lock(&p.downloads).clone())
            .filter(|d| d.session.as_deref() == Some(scope.session_id.as_str()))
            .collect()
    };
    let fmt = |ds: &[crate::profile::Download]| -> String {
        ds.iter()
            .map(|d| {
                format!(
                    "{} · {} · {}",
                    d.name,
                    d.state,
                    d.path
                        .as_ref()
                        .map_or_else(|| "(in progress)".into(), |p| p.display().to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    match action {
        DownloadsAction::List => {
            let ds = list();
            BrowserResult::ok(if ds.is_empty() {
                "No downloads in this conversation.".into()
            } else {
                fmt(&ds)
            })
        }
        DownloadsAction::Wait => {
            let limit = Duration::from_millis(timeout_ms.unwrap_or(30_000).min(120_000));
            let start = Instant::now();
            loop {
                let mut done = Vec::new();
                for p in lock(&svc.inner.live).values() {
                    for d in lock(&p.downloads).iter_mut() {
                        if d.session.as_deref() == Some(scope.session_id.as_str())
                            && d.state == "completed"
                            && !d.reported
                        {
                            d.reported = true;
                            done.push(d.clone());
                        }
                    }
                }
                if !done.is_empty() {
                    return BrowserResult::ok(format!("Downloaded:\n{}", fmt(&done)));
                }
                if start.elapsed() > limit {
                    return BrowserResult::error(
                        BrowserErrorCode::Timeout,
                        "no download completed in time",
                        SideEffect::None,
                    );
                }
                let notifies: Vec<Arc<ProfileRt>> =
                    lock(&svc.inner.live).values().cloned().collect();
                let wake = async {
                    if let Some(p) = notifies.first() {
                        p.download_done.notified().await;
                    } else {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                };
                tokio::select! {
                    () = wake => {},
                    () = tokio::time::sleep(Duration::from_millis(250)) => {},
                    () = cancel.cancelled() => return BrowserResult::error(BrowserErrorCode::Cancelled, "cancelled", SideEffect::None),
                }
            }
        }
    }
}

fn profile_op(
    svc: &BrowserService,
    scope: &BrowserScope,
    action: ProfileAction,
    name: Option<&str>,
    approval: Approval,
) -> BrowserResult {
    match action {
        ProfileAction::Status => {
            let st = svc.status_for_scope(scope);
            let mut text = format!(
                "{} {} · engine {} · stealth {}{} · display {} · session profile {}\nProfiles:",
                st.product.as_deref().unwrap_or("(no browser)"),
                st.version.as_deref().unwrap_or(""),
                st.engine.as_deref().unwrap_or("none"),
                if st.stealth { "yes" } else { "no" },
                st.stealth_note
                    .as_ref()
                    .map_or_else(String::new, |n| format!(" ({n})")),
                st.display,
                st.session_profile.as_deref().unwrap_or("default"),
            );
            for p in &st.profiles {
                let _ = write!(
                    text,
                    "\n- {}{}{}",
                    p.name,
                    if p.running { " (running)" } else { "" },
                    if p.cookie_domains.is_empty() {
                        String::new()
                    } else {
                        format!(" · logins: {}", p.cookie_domains.join(", "))
                    }
                );
            }
            let taken: Vec<String> = st
                .tabs
                .iter()
                .filter(|t| t.controlled_by == ControlledBy::User)
                .map(|t| t.id.clone())
                .collect();
            if !taken.is_empty() {
                let _ = write!(text, "\nTaken over by the user: {}", taken.join(", "));
            }
            BrowserResult {
                text,
                details: BrowserToolDetails {
                    data: serde_json::to_value(&st).ok(),
                    ..BrowserToolDetails::default()
                },
                ..BrowserResult::default()
            }
        }
        ProfileAction::Use => {
            let Some(name) = name else {
                return BrowserResult::error(
                    BrowserErrorCode::ExecutionError,
                    "pass name",
                    SideEffect::None,
                );
            };
            if !svc.profiles().iter().any(|p| p.name == name) {
                return BrowserResult::error(
                    BrowserErrorCode::ExecutionError,
                    format!(
                        "profile {name} does not exist; browser_profile {{action:\"create\"}} first"
                    ),
                    SideEffect::None,
                );
            }
            if let Some(s) = lock(&svc.hub().sessions).get_mut(&scope.session_id) {
                s.profile = Some(name.to_owned());
            }
            BrowserResult::ok(format!(
                "This conversation now opens new tabs in profile {name}"
            ))
        }
        ProfileAction::Create => {
            let Some(name) = name else {
                return BrowserResult::error(
                    BrowserErrorCode::ExecutionError,
                    "pass name",
                    SideEffect::None,
                );
            };
            if approval == Approval::None {
                return BrowserResult::error(
                    BrowserErrorCode::ApprovalRequired,
                    format!("Agent wants to create browser profile {name}"),
                    SideEffect::None,
                );
            }
            match crate::profiles::create(&svc.hub().paths, name) {
                Ok(()) => BrowserResult::ok(format!("Created profile {name}")),
                Err(e) => {
                    BrowserResult::error(BrowserErrorCode::ExecutionError, e, SideEffect::None)
                }
            }
        }
    }
}

async fn session_closed(svc: &BrowserService, session: &str, close_tabs: bool) {
    let profiles: Vec<Arc<ProfileRt>> = lock(&svc.inner.live).values().cloned().collect();
    for p in profiles {
        for t in p.all_tabs() {
            let mine = lock(&t.state).owner.session() == Some(session);
            if !mine {
                continue;
            }
            if close_tabs {
                p.close_tab(&t, "agent").await;
            } else {
                let mut s = lock(&t.state);
                s.owner = TabOwner::User {
                    session: Some(session.to_owned()),
                };
                s.lifecycle = TabLifecycle::User;
            }
        }
    }
    svc.hub().set_current(session, None);
}

async fn close_agent_tabs(svc: &BrowserService, session: &str) -> usize {
    let profiles: Vec<Arc<ProfileRt>> = lock(&svc.inner.live).values().cloned().collect();
    let mut n = 0;
    for p in profiles {
        for t in p.all_tabs() {
            let agent =
                matches!(&lock(&t.state).owner, TabOwner::Agent { session: s } if s == session);
            if agent {
                p.close_tab(&t, "agent").await;
                n += 1;
            }
        }
    }
    n
}

fn base_details(cx: &Cx<'_>, tab: Option<&Tab>) -> BrowserDetails {
    let open: Vec<String> = cx.agent_tabs().iter().map(|t| t.id.clone()).collect();
    let mut d = BrowserDetails {
        profile: cx.rt.name.clone(),
        engine: cx.rt.engine.clone(),
        generation: cx.rt.generation,
        open_tabs: open,
        ..BrowserDetails::default()
    };
    if let Some(t) = tab {
        let s = lock(&t.state);
        d.tab = Some(t.id.clone());
        d.url = Some(redact_url(&s.url));
        d.title = Some(s.title.clone());
        d.lifecycle = Some(s.lifecycle.as_str().to_owned());
    }
    d
}

async fn success(cx: &Cx<'_>, cmd: &BrowserCommand, mut o: Outcome) -> BrowserResult {
    let events: Vec<ToolEvent> = o
        .tab
        .as_ref()
        .and_then(|t| lock(&t.state).collecting.take())
        .unwrap_or_default();
    let mut text = std::mem::take(&mut o.text);
    let lines: Vec<String> = events
        .iter()
        .map(|e| match e {
            ToolEvent::Popup { tab } => format!("opened tab {tab} (popup)"),
            ToolEvent::Navigated { url } => format!("navigated to {url}"),
            ToolEvent::Dialog { dialog, message } => {
                format!("{dialog} dialog opened: \"{message}\" (answer with browser_handle_dialog)")
            }
            ToolEvent::Download { name } => {
                format!("download started: {name} (browser_downloads wait)")
            }
            ToolEvent::Errors { count } => format!("{count} page error(s) (browser_logs errors)"),
        })
        .collect();
    let lines: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            !(matches!(cmd, BrowserCommand::Navigate { .. }) && l.starts_with("navigated to"))
        })
        .collect();
    if !lines.is_empty() {
        let _ = write!(text, " · {}", lines.join(" · "));
    }
    if o.want_snapshot
        && let Some(t) = &o.tab
        && !matches!(cmd, BrowserCommand::Snapshot { .. })
        && lock(&t.state).dialog.is_none()
    {
        match page::take_snapshot(&cx.rt.cdp, t, None, 24_000, true).await {
            Ok((snap, sid)) => {
                text.push('\n');
                text.push_str(&snap);
                o.snapshot_id = Some(sid);
            }
            Err(e) => {
                let _ = write!(text, "\n(snapshot failed: {})", e.message);
            }
        }
    }
    let mut d = base_details(cx, o.tab.as_deref());
    d.snapshot_id = o.snapshot_id;
    d.events = events;
    d.side_effect = if cmd.is_driving() {
        SideEffect::Done
    } else {
        SideEffect::None
    };
    BrowserResult {
        text,
        details: BrowserToolDetails {
            browser: Some(d),
            error: None,
            data: o.data,
        },
        is_error: false,
        image: o.image,
    }
}
