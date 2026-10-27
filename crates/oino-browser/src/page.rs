//! CDP operations on one tab (browser-control.md §7): snapshot fetching across frames, ref
//! resolution, actionability, trusted input, isolated-world evaluation, screenshots.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use oino_browser_protocol::{BrowserErrorCode, MouseButton, SideEffect};
use serde_json::{Value, json};

use crate::cdp::{Cdp, CdpError};
use crate::keys::Chord;
use crate::snapshot::{self, FrameTree, RefEntry, RenderOptions};
use crate::tab::{Tab, lock};

/// Routine actionability timeout (§5.1).
pub(crate) const ROUTINE: Duration = Duration::from_millis(3000);

/// A failed action.
#[derive(Clone, Debug)]
pub(crate) struct ActionError {
    pub(crate) code: BrowserErrorCode,
    pub(crate) message: String,
    pub(crate) side: SideEffect,
}

impl ActionError {
    pub(crate) fn new(code: BrowserErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            side: SideEffect::None,
        }
    }

    pub(crate) fn uncertain(mut self) -> Self {
        self.side = SideEffect::Uncertain;
        self
    }
}

impl From<CdpError> for ActionError {
    fn from(e: CdpError) -> Self {
        match e {
            CdpError::Closed => Self::new(
                BrowserErrorCode::BackendUnavailable,
                "the browser closed; retry to restart it",
            ),
            CdpError::Timeout(m) => Self::new(
                BrowserErrorCode::Timeout,
                format!("{m} did not answer; the page may be busy — retry or take a new snapshot"),
            ),
            CdpError::Protocol { method, message } => Self::new(
                BrowserErrorCode::ExecutionError,
                format!("{method}: {message}"),
            ),
        }
    }
}

pub(crate) type ActionResult<T> = Result<T, ActionError>;

/// Page metrics for the snapshot header.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Layout {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) scroll_x: f64,
    pub(crate) scroll_y: f64,
    pub(crate) content_height: f64,
    pub(crate) content_width: f64,
}

pub(crate) async fn layout(cdp: &Cdp, tab: &Tab) -> ActionResult<Layout> {
    let m = cdp
        .call(Some(&tab.session), "Page.getLayoutMetrics", json!({}))
        .await?;
    let v = &m["cssVisualViewport"];
    let c = &m["cssContentSize"];
    Ok(Layout {
        width: v["clientWidth"].as_f64().unwrap_or(0.0),
        height: v["clientHeight"].as_f64().unwrap_or(0.0),
        scroll_x: v["pageX"].as_f64().unwrap_or(0.0),
        scroll_y: v["pageY"].as_f64().unwrap_or(0.0),
        content_height: c["height"].as_f64().unwrap_or(0.0),
        content_width: c["width"].as_f64().unwrap_or(0.0),
    })
}

fn frame_ids(tree: &Value, out: &mut Vec<(String, String)>) {
    let id = tree["frame"]["id"].as_str().unwrap_or_default().to_owned();
    if let Some(children) = tree["childFrames"].as_array() {
        for c in children {
            let cid = c["frame"]["id"].as_str().unwrap_or_default().to_owned();
            out.push((cid, id.clone()));
            frame_ids(c, out);
        }
    }
}

/// Rounds a CSS pixel value for integer CDP parameters.
#[allow(
    clippy::cast_possible_truncation,
    reason = "viewport coordinates and sizes are far inside i64"
)]
pub(crate) fn px(v: f64) -> i64 {
    v.round() as i64
}

/// OOPIF owner chain (see `FrameTree::chain`).
type Chain = Vec<(String, i64)>;

/// Fetches the AX forest of a tab (frames merged at owners; budgets 500 ms / frame,
/// 1000 ms for all child frames).
#[allow(
    clippy::too_many_lines,
    reason = "one step list per operation, clearer kept together"
)]
pub(crate) async fn forest(cdp: &Cdp, tab: &Tab, frames: bool) -> ActionResult<snapshot::Forest> {
    let main = cdp
        .call_timeout(
            Some(&tab.session),
            "Accessibility.getFullAXTree",
            json!({}),
            Duration::from_secs(10),
        )
        .await?;
    let mut forest = snapshot::Forest::default();
    forest.frames.push(FrameTree {
        session: tab.session.clone(),
        frame_id: tab.target_id.clone(),
        nodes: snapshot::parse_nodes(&main),
        chain: Vec::new(),
    });
    if !frames {
        return Ok(forest);
    }
    let children = lock(&tab.state).children.clone();
    let deadline = Instant::now() + Duration::from_millis(1000);
    let per = Duration::from_millis(500);
    // Work list of sessions: (session, root frame id, chain).
    let mut queue: Vec<(String, String, Chain)> =
        vec![(tab.session.clone(), tab.target_id.clone(), Vec::new())];
    let mut first = true;
    while let Some((session, root, chain)) = queue.pop() {
        if Instant::now() > deadline {
            break;
        }
        if !first
            && let Ok(t) = cdp
                .call_timeout(
                    Some(&session),
                    "Accessibility.getFullAXTree",
                    json!({}),
                    per,
                )
                .await
        {
            forest.frames.push(FrameTree {
                session: session.clone(),
                frame_id: root.clone(),
                nodes: snapshot::parse_nodes(&t),
                chain: chain.clone(),
            });
        }
        first = false;
        let mut same = Vec::new();
        if let Ok(tree) = cdp
            .call_timeout(Some(&session), "Page.getFrameTree", json!({}), per)
            .await
        {
            frame_ids(&tree["frameTree"], &mut same);
        }
        for (fid, _) in same {
            if children.iter().any(|c| c.target_id == fid) || Instant::now() > deadline {
                continue;
            }
            let owner = cdp
                .call_timeout(
                    Some(&session),
                    "DOM.getFrameOwner",
                    json!({"frameId": fid}),
                    per,
                )
                .await
                .ok()
                .and_then(|o| o["backendNodeId"].as_i64());
            let Some(owner) = owner else { continue };
            if let Ok(t) = cdp
                .call_timeout(
                    Some(&session),
                    "Accessibility.getFullAXTree",
                    json!({"frameId": fid}),
                    per,
                )
                .await
            {
                forest
                    .owners
                    .insert((session.clone(), owner), forest.frames.len());
                forest.frames.push(FrameTree {
                    session: session.clone(),
                    frame_id: fid,
                    nodes: snapshot::parse_nodes(&t),
                    chain: chain.clone(),
                });
            }
        }
        for c in children
            .iter()
            .filter(|c| c.kind == "iframe" && c.parent_session == session)
        {
            let owner = cdp
                .call_timeout(
                    Some(&session),
                    "DOM.getFrameOwner",
                    json!({"frameId": c.target_id}),
                    per,
                )
                .await
                .ok()
                .and_then(|o| o["backendNodeId"].as_i64());
            if let Some(owner) = owner {
                forest
                    .owners
                    .insert((session.clone(), owner), forest.frames.len());
                let mut ch = chain.clone();
                ch.push((session.clone(), owner));
                queue.insert(0, (c.session.clone(), c.target_id.clone(), ch));
            }
        }
    }
    // Owners recorded for OOPIFs point at the index the frame got when fetched; re-map by
    // session to be robust against fetch failures.
    let by_session: HashMap<(String, String), usize> = forest
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| ((f.session.clone(), f.frame_id.clone()), i))
        .collect();
    let mut fixed = HashMap::new();
    for ((s, owner), idx) in &forest.owners {
        let target = forest
            .frames
            .get(*idx)
            .filter(|f| f.chain.last() == Some(&(s.clone(), *owner)) || f.session == *s)
            .map(|_| *idx)
            .or_else(|| {
                children
                    .iter()
                    .find(|c| c.parent_session == *s)
                    .and_then(|c| {
                        by_session
                            .get(&(c.session.clone(), c.target_id.clone()))
                            .copied()
                    })
            });
        if let Some(t) = target {
            fixed.insert((s.clone(), *owner), t);
        }
    }
    forest.owners = fixed;
    Ok(forest)
}

/// Refreshes URL and title from `Target.getTargetInfo`.
pub(crate) async fn refresh_info(cdp: &Cdp, tab: &Tab) {
    if let Ok(r) = cdp
        .call(
            None,
            "Target.getTargetInfo",
            json!({"targetId": tab.target_id}),
        )
        .await
    {
        let info = &r["targetInfo"];
        let mut s = lock(&tab.state);
        if let Some(u) = info["url"].as_str() {
            u.clone_into(&mut s.url);
        }
        if let Some(t) = info["title"].as_str() {
            s.title = clean_title(t, &s.url);
        }
    }
}

/// Chrome reports the URL (minus scheme) as the title of untitled pages.
pub(crate) fn clean_title(title: &str, url: &str) -> String {
    let bare = url.split_once("://").map_or(url, |(_, r)| r);
    if title == url || title == bare || title.is_empty() {
        String::new()
    } else {
        title.to_owned()
    }
}

/// Takes a snapshot; returns `(text, snapshot id, truncated)`.
pub(crate) async fn take_snapshot(
    cdp: &Cdp,
    tab: &Tab,
    scope: Option<&str>,
    max_chars: usize,
    frames: bool,
) -> ActionResult<(String, String)> {
    let scope_entry = match scope {
        Some(r) => Some(resolve_ref(tab, r)?),
        None => None,
    };
    let forest = forest(cdp, tab, frames).await?;
    refresh_info(cdp, tab).await;
    let l = layout(cdp, tab).await.unwrap_or_default();
    let (url, title, seq, text, truncated) = {
        let mut s = lock(&tab.state);
        s.snapshot_seq += 1;
        let seq = s.snapshot_seq;
        let opts = RenderOptions {
            max_chars,
            snapshot: seq,
            scope: scope_entry,
        };
        let (text, truncated) = snapshot::render(&forest, &mut s.refs, &opts);
        (s.url.clone(), s.title.clone(), seq, text, truncated)
    };
    let sid = format!("s{seq}");
    let mut header = format!(
        "# {} · {} · {} · {} · viewport {:.0}×{:.0} · scroll {:.0},{:.0}/{:.0}",
        tab.id,
        if title.is_empty() {
            "(untitled)"
        } else {
            &title
        },
        url,
        sid,
        l.width,
        l.height,
        l.scroll_x,
        l.scroll_y,
        l.content_height
    );
    if truncated {
        header.push_str(" · truncated — use scope=<ref> or scroll");
    }
    let body = if text.is_empty() {
        "(empty page)\n".to_owned()
    } else {
        text
    };
    Ok((format!("{header}\n{body}"), sid))
}

/// Looks up a ref of this tab.
pub(crate) fn resolve_ref(tab: &Tab, r: &str) -> ActionResult<RefEntry> {
    let r = r.trim().trim_start_matches('@').trim_start_matches("ref=");
    lock(&tab.state).refs.get(r).cloned().ok_or_else(|| {
        ActionError::new(
            BrowserErrorCode::RefNotFound,
            format!(
                "{r} is not a ref of tab {}; take a snapshot and use its refs",
                tab.id
            ),
        )
    })
}

/// Resolves a ref to a live JS object (main world), else `ref_stale`.
pub(crate) async fn object_of(cdp: &Cdp, r: &str, e: &RefEntry) -> ActionResult<String> {
    let reply = cdp
        .call(
            Some(&e.session),
            "DOM.resolveNode",
            json!({"backendNodeId": e.backend, "objectGroup": "oino"}),
        )
        .await
        .map_err(|_| stale(r, e))?;
    reply["object"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| stale(r, e))
}

fn stale(r: &str, e: &RefEntry) -> ActionError {
    ActionError::new(
        BrowserErrorCode::RefStale,
        format!(
            "{r} ({} \"{}\") is from snapshot s{} and the element is gone; take a new snapshot",
            e.role, e.name, e.snapshot
        ),
    )
}

/// `Runtime.callFunctionOn` on an object, returning the value.
pub(crate) async fn call_on(
    cdp: &Cdp,
    session: &str,
    object: &str,
    function: &str,
    args: &[Value],
) -> ActionResult<Value> {
    let arguments: Vec<Value> = args.iter().map(|a| json!({"value": a})).collect();
    let r = cdp
        .call(
            Some(session),
            "Runtime.callFunctionOn",
            json!({"objectId": object, "functionDeclaration": function, "arguments": arguments,
                   "returnByValue": true, "awaitPromise": true}),
        )
        .await?;
    if let Some(ex) = r.get("exceptionDetails") {
        return Err(ActionError::new(
            BrowserErrorCode::ExecutionError,
            exception_text(ex),
        ));
    }
    Ok(r["result"]["value"].clone())
}

pub(crate) fn exception_text(ex: &Value) -> String {
    ex["exception"]["description"]
        .as_str()
        .or_else(|| ex["text"].as_str())
        .unwrap_or("exception")
        .lines()
        .next()
        .unwrap_or("exception")
        .to_owned()
}

async fn quads(cdp: &Cdp, e: &RefEntry) -> Option<Vec<f64>> {
    let r = cdp
        .call(
            Some(&e.session),
            "DOM.getContentQuads",
            json!({"backendNodeId": e.backend}),
        )
        .await
        .ok()?;
    let q = r["quads"].as_array()?.first()?.as_array()?;
    let v: Vec<f64> = q.iter().filter_map(Value::as_f64).collect();
    (v.len() == 8).then_some(v)
}

fn area(q: &[f64]) -> f64 {
    let mut a = 0.0;
    for i in 0..4 {
        let (x1, y1) = (q[i * 2], q[i * 2 + 1]);
        let (x2, y2) = (q[(i * 2 + 2) % 8], q[(i * 2 + 3) % 8]);
        a += x1 * y2 - x2 * y1;
    }
    (a / 2.0).abs()
}

async fn raf(cdp: &Cdp, session: &str) {
    let _ = cdp
        .call_timeout(
            Some(session),
            "Runtime.evaluate",
            json!({"expression": "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))",
                   "awaitPromise": true}),
            Duration::from_millis(500),
        )
        .await;
}

/// Offset of an OOPIF chain (sum of owners' content-box origins).
async fn chain_offset(cdp: &Cdp, chain: &[(String, i64)]) -> ActionResult<(f64, f64)> {
    let mut off = (0.0, 0.0);
    for (session, owner) in chain {
        let b = cdp
            .call(
                Some(session),
                "DOM.getBoxModel",
                json!({"backendNodeId": owner}),
            )
            .await?;
        let c = &b["model"]["content"];
        off.0 += c[0].as_f64().unwrap_or(0.0);
        off.1 += c[1].as_f64().unwrap_or(0.0);
    }
    Ok(off)
}

const CONTAINS: &str =
    "function(o){let n=o;while(n){if(n===this)return true;n=n.parentNode||n.host;}return false}";
const DESCRIBE: &str = "function(){const e=this.nodeType===1?this:this.parentElement;if(!e)return 'element';const r=e.getAttribute('role')||e.tagName.toLowerCase();const n=e.getAttribute('aria-label')||e.id||(e.innerText||'').trim().slice(0,40);return r+(n?' \"'+n+'\"':'')}";
const ENABLED: &str = "function(){const e=this.nodeType===1?this:this.parentElement;if(!e)return true;if(e.disabled)return false;if(e.closest('[aria-disabled=\"true\"],fieldset[disabled]'))return false;return getComputedStyle(e).visibility!=='hidden'}";

/// Actionability (§7.2 steps 1–5): returns the top-level viewport point to click.
pub(crate) async fn actionable_point(
    cdp: &Cdp,
    tab: &Tab,
    r: &str,
    e: &RefEntry,
    need_enabled: bool,
) -> ActionResult<(f64, f64)> {
    let object = object_of(cdp, r, e).await?;
    let _ = cdp
        .call(
            Some(&e.session),
            "DOM.scrollIntoViewIfNeeded",
            json!({"backendNodeId": e.backend}),
        )
        .await;
    let deadline = Instant::now() + ROUTINE;
    let mut last = ActionError::new(
        BrowserErrorCode::NotVisible,
        format!(
            "{r} ({} \"{}\") is not visible; scroll or take a new snapshot",
            e.role, e.name
        ),
    );
    loop {
        let a = quads(cdp, e).await;
        raf(cdp, &e.session).await;
        let b = quads(cdp, e).await;
        match (a, b) {
            (Some(a), Some(b)) if a == b && area(&a) > 0.5 => {
                if need_enabled
                    && call_on(cdp, &e.session, &object, ENABLED, &[]).await? == json!(false)
                {
                    last = ActionError::new(
                        BrowserErrorCode::NotEnabled,
                        format!(
                            "{r} ({} \"{}\") is disabled; wait for it or pick another element",
                            e.role, e.name
                        ),
                    );
                } else {
                    let local = (
                        (a[0] + a[2] + a[4] + a[6]) / 4.0,
                        (a[1] + a[3] + a[5] + a[7]) / 4.0,
                    );
                    let off = chain_offset(cdp, &e.chain).await?;
                    let point = (local.0 + off.0, local.1 + off.1);
                    match hit_ok(cdp, tab, e, &object, point).await? {
                        None => return Ok(point),
                        Some(cover) => {
                            last = ActionError::new(
                                BrowserErrorCode::Obscured,
                                format!(
                                    "{r} ({} \"{}\") is covered by {cover}; close it (dialog, banner) or scroll, then retry",
                                    e.role, e.name
                                ),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        if Instant::now() > deadline {
            return Err(last);
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

/// `None` when the hit target at `point` is the element (or inside it); else a description of
/// the covering element.
async fn hit_ok(
    cdp: &Cdp,
    tab: &Tab,
    e: &RefEntry,
    object: &str,
    point: (f64, f64),
) -> ActionResult<Option<String>> {
    let mut session = tab.session.clone();
    let mut p = point;
    for (i, (owner_session, owner)) in e.chain.iter().enumerate() {
        let hit = node_at(cdp, &session, p).await?;
        if hit != Some(*owner) {
            return describe_hit(cdp, &session, hit).await.map(Some);
        }
        let off = chain_offset(cdp, std::slice::from_ref(&(owner_session.clone(), *owner))).await?;
        p = (p.0 - off.0, p.1 - off.1);
        session = e
            .chain
            .get(i + 1)
            .map_or_else(|| e.session.clone(), |next| next.0.clone());
    }
    let hit = node_at(cdp, &session, p).await?;
    let Some(hit_id) = hit else {
        return Ok(Some("nothing (outside the page)".into()));
    };
    if hit_id == e.backend {
        return Ok(None);
    }
    let hit_obj = cdp
        .call(
            Some(&session),
            "DOM.resolveNode",
            json!({"backendNodeId": hit_id, "objectGroup": "oino"}),
        )
        .await?;
    let Some(hit_obj) = hit_obj["object"]["objectId"].as_str() else {
        return Ok(None);
    };
    let inside = cdp
        .call(
            Some(&e.session),
            "Runtime.callFunctionOn",
            json!({"objectId": object, "functionDeclaration": CONTAINS,
                   "arguments": [{"objectId": hit_obj}], "returnByValue": true}),
        )
        .await?;
    if inside["result"]["value"] == json!(true) {
        return Ok(None);
    }
    describe_hit(cdp, &session, Some(hit_id)).await.map(Some)
}

async fn node_at(cdp: &Cdp, session: &str, p: (f64, f64)) -> ActionResult<Option<i64>> {
    let r = cdp
        .call(
            Some(session),
            "DOM.getNodeForLocation",
            json!({"x": px(p.0), "y": px(p.1),
                   "includeUserAgentShadowDOM": true, "ignorePointerEventsNone": true}),
        )
        .await;
    Ok(r.ok().and_then(|r| r["backendNodeId"].as_i64()))
}

async fn describe_hit(cdp: &Cdp, session: &str, hit: Option<i64>) -> ActionResult<String> {
    let Some(id) = hit else {
        return Ok("nothing".into());
    };
    let o = cdp
        .call(
            Some(session),
            "DOM.resolveNode",
            json!({"backendNodeId": id, "objectGroup": "oino"}),
        )
        .await?;
    let Some(obj) = o["object"]["objectId"].as_str() else {
        return Ok("another element".into());
    };
    Ok(call_on(cdp, session, obj, DESCRIBE, &[])
        .await?
        .as_str()
        .unwrap_or("another element")
        .to_owned())
}

/// Human-like pointer movement settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Human {
    /// Direct events.
    Off,
    /// Bézier paths and key timing.
    On,
    /// Slower.
    Careful,
}

fn button_name(b: MouseButton) -> (&'static str, u32) {
    match b {
        MouseButton::Left => ("left", 1),
        MouseButton::Right => ("right", 2),
        MouseButton::Middle => ("middle", 4),
    }
}

/// Moves the pointer to `to` (Bézier path when humanized; port of unipi `humanize.rs` idea).
pub(crate) async fn move_to(
    cdp: &Cdp,
    tab: &Tab,
    to: (f64, f64),
    human: Human,
    modifiers: u8,
) -> ActionResult<()> {
    let from = lock(&tab.state).pointer;
    let steps: u32 = match human {
        Human::Off => 1,
        Human::On => 12,
        Human::Careful => 24,
    };
    let ctrl = (
        from.0 + (to.0 - from.0) * 0.3 + (to.1 - from.1) * 0.15,
        from.1 + (to.1 - from.1) * 0.3 - (to.0 - from.0) * 0.15,
    );
    for i in 1..=steps {
        let t = f64::from(i) / f64::from(steps);
        let (x, y) = if steps == 1 {
            to
        } else {
            let u = 1.0 - t;
            (
                u * u * from.0 + 2.0 * u * t * ctrl.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * ctrl.1 + t * t * to.1,
            )
        };
        cdp.call(
            Some(&tab.session),
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y, "modifiers": modifiers}),
        )
        .await?;
        if steps > 1 {
            tokio::time::sleep(Duration::from_millis(if human == Human::Careful {
                14
            } else {
                8
            }))
            .await;
        }
    }
    lock(&tab.state).pointer = to;
    Ok(())
}

/// Dispatches an input event; a JS dialog opened by it blocks the CDP reply until answered,
/// so the call counts as done as soon as a dialog appears.
pub(crate) async fn input_call(
    cdp: &Cdp,
    tab: &Tab,
    method: &str,
    params: Value,
) -> ActionResult<()> {
    let call = cdp.call(Some(&tab.session), method, params);
    tokio::pin!(call);
    loop {
        let changed = tab.changed.notified();
        tokio::select! {
            r = &mut call => return r.map(|_| ()).map_err(|e| ActionError::from(e).uncertain()),
            () = changed => {
                if lock(&tab.state).dialog.is_some() {
                    return Ok(());
                }
            }
        }
    }
}

/// Click at `p`.
pub(crate) async fn click_at(
    cdp: &Cdp,
    tab: &Tab,
    p: (f64, f64),
    button: MouseButton,
    count: u32,
    modifiers: u8,
    human: Human,
) -> ActionResult<()> {
    move_to(cdp, tab, p, human, modifiers).await?;
    let (name, bits) = button_name(button);
    for n in 1..=count {
        for kind in ["mousePressed", "mouseReleased"] {
            input_call(
                cdp,
                tab,
                "Input.dispatchMouseEvent",
                json!({"type": kind, "x": p.0, "y": p.1, "button": name,
                       "buttons": if kind == "mousePressed" { bits } else { 0 },
                       "clickCount": n, "modifiers": modifiers}),
            )
            .await?;
            if human != Human::Off && kind == "mousePressed" {
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
        }
    }
    Ok(())
}

/// Press, ≥ 8 intermediate moves, release.
pub(crate) async fn drag(
    cdp: &Cdp,
    tab: &Tab,
    from: (f64, f64),
    to: (f64, f64),
) -> ActionResult<()> {
    move_to(cdp, tab, from, Human::Off, 0).await?;
    let s = Some(tab.session.as_str());
    cdp.call(
        s,
        "Input.dispatchMouseEvent",
        json!({"type": "mousePressed", "x": from.0, "y": from.1, "button": "left", "buttons": 1, "clickCount": 1}),
    )
    .await?;
    for i in 1..=10 {
        let t = f64::from(i) / 10.0;
        let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        cdp.call(
            s,
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y, "button": "left", "buttons": 1}),
        )
        .await
        .map_err(|e| ActionError::from(e).uncertain())?;
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
    cdp.call(
        s,
        "Input.dispatchMouseEvent",
        json!({"type": "mouseReleased", "x": to.0, "y": to.1, "button": "left", "buttons": 0, "clickCount": 1}),
    )
    .await
    .map_err(|e| ActionError::from(e).uncertain())?;
    lock(&tab.state).pointer = to;
    Ok(())
}

/// Key down/up for a chord.
pub(crate) async fn press(cdp: &Cdp, tab: &Tab, chord: &Chord) -> ActionResult<()> {
    let s = Some(tab.session.as_str());
    let mut bits = 0u8;
    for (k, bit) in chord.held.iter().zip(mod_bits(chord.modifiers)) {
        bits |= bit;
        cdp.call(
            s,
            "Input.dispatchKeyEvent",
            json!({"type": "rawKeyDown", "key": k.key, "code": k.code,
                   "windowsVirtualKeyCode": k.key_code, "modifiers": bits}),
        )
        .await?;
    }
    let k = &chord.key;
    let mut down = json!({"type": if k.text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.key_code, "modifiers": chord.modifiers});
    if let Some(t) = &k.text {
        down["text"] = json!(t);
        down["unmodifiedText"] = json!(t);
    }
    input_call(cdp, tab, "Input.dispatchKeyEvent", down).await?;
    input_call(
        cdp,
        tab,
        "Input.dispatchKeyEvent",
        json!({"type": "keyUp", "key": k.key, "code": k.code,
               "windowsVirtualKeyCode": k.key_code, "modifiers": chord.modifiers}),
    )
    .await?;
    for k in chord.held.iter().rev() {
        let _ = cdp
            .call(
                s,
                "Input.dispatchKeyEvent",
                json!({"type": "keyUp", "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.key_code}),
            )
            .await;
    }
    Ok(())
}

fn mod_bits(bits: u8) -> Vec<u8> {
    [1u8, 2, 4, 8]
        .into_iter()
        .filter(|b| bits & b != 0)
        .collect()
}

/// Inserts text (per-character key events when humanized).
pub(crate) async fn insert_text(
    cdp: &Cdp,
    tab: &Tab,
    text: &str,
    human: Human,
) -> ActionResult<()> {
    let session = tab.session.as_str();
    if human == Human::Off {
        input_call(cdp, tab, "Input.insertText", json!({"text": text})).await?;
        return Ok(());
    }
    for c in text.chars() {
        if c.is_ascii_graphic() || c == ' ' {
            let chord = Chord {
                modifiers: 0,
                held: Vec::new(),
                key: crate::keys::char_def(c),
            };
            press(cdp, tab, &chord).await?;
        } else {
            cdp.call(
                Some(session),
                "Input.insertText",
                json!({"text": c.to_string()}),
            )
            .await?;
        }
        let ms = if human == Human::Careful { 90 } else { 35 };
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    Ok(())
}

/// Creates an isolated world in `frame` and returns its execution context.
pub(crate) async fn isolated_world(cdp: &Cdp, session: &str, frame: &str) -> ActionResult<i64> {
    let r = cdp
        .call(
            Some(session),
            "Page.createIsolatedWorld",
            json!({"frameId": frame, "worldName": "oino", "grantUniveralAccess": false}),
        )
        .await?;
    r["executionContextId"]
        .as_i64()
        .ok_or_else(|| ActionError::new(BrowserErrorCode::ExecutionError, "no isolated world"))
}

/// Evaluates `function` in an isolated world of the tab's main frame (or the ref's frame).
pub(crate) async fn evaluate(
    cdp: &Cdp,
    tab: &Tab,
    function: &str,
    target: Option<(&str, &RefEntry)>,
    await_promise: bool,
) -> ActionResult<Value> {
    let (session, frame) = match target {
        Some((_, e)) => (e.session.clone(), e.frame_id.clone()),
        None => (tab.session.clone(), tab.target_id.clone()),
    };
    let ctx = isolated_world(cdp, &session, &frame).await?;
    let mut params = json!({"functionDeclaration": function, "returnByValue": true,
                            "awaitPromise": await_promise, "executionContextId": ctx});
    if let Some((r, e)) = target {
        let o = cdp
            .call(
                Some(&session),
                "DOM.resolveNode",
                json!({"backendNodeId": e.backend, "executionContextId": ctx}),
            )
            .await
            .map_err(|_| stale(r, e))?;
        let obj = o["object"]["objectId"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        params["arguments"] = json!([{"objectId": obj}]);
    }
    let r = cdp
        .call(Some(&session), "Runtime.callFunctionOn", params)
        .await
        .map_err(|e| ActionError::from(e).uncertain())?;
    if let Some(ex) = r.get("exceptionDetails") {
        return Err(ActionError::new(
            BrowserErrorCode::ExecutionError,
            format!("the function threw: {}", exception_text(ex)),
        )
        .uncertain());
    }
    Ok(r["result"].get("value").cloned().unwrap_or_else(|| {
        r["result"]["description"]
            .as_str()
            .map_or(Value::Null, |d| Value::String(d.to_owned()))
    }))
}

/// Evaluates an expression in an isolated world of the main frame (waits, probes).
pub(crate) async fn probe(cdp: &Cdp, tab: &Tab, expression: &str) -> ActionResult<Value> {
    evaluate(cdp, tab, &format!("() => ({expression})"), None, true).await
}

/// PNG screenshot (viewport, full page, or an element's box).
pub(crate) async fn screenshot_png(
    cdp: &Cdp,
    tab: &Tab,
    clip: Option<(f64, f64, f64, f64)>,
    full_page: bool,
) -> ActionResult<Vec<u8>> {
    use base64::Engine as _;
    let mut params = json!({"format": "png", "fromSurface": true});
    if let Some((x, y, w, h)) = clip {
        let l = layout(cdp, tab).await.unwrap_or_default();
        params["clip"] =
            json!({"x": x + l.scroll_x, "y": y + l.scroll_y, "width": w, "height": h, "scale": 1});
        params["captureBeyondViewport"] = json!(true);
    } else if full_page {
        let l = layout(cdp, tab).await?;
        params["clip"] = json!({"x": 0, "y": 0, "width": l.content_width.max(1.0),
                                "height": l.content_height.clamp(1.0, 16_384.0), "scale": 1});
        params["captureBeyondViewport"] = json!(true);
    }
    let r = cdp
        .call_timeout(
            Some(&tab.session),
            "Page.captureScreenshot",
            params,
            Duration::from_secs(20),
        )
        .await?;
    base64::engine::general_purpose::STANDARD
        .decode(r["data"].as_str().unwrap_or_default())
        .map_err(|e| ActionError::new(BrowserErrorCode::ExecutionError, e.to_string()))
}

/// Element bounding box in top-level viewport coordinates.
pub(crate) async fn element_box(
    cdp: &Cdp,
    r: &str,
    e: &RefEntry,
) -> ActionResult<(f64, f64, f64, f64)> {
    let _ = cdp
        .call(
            Some(&e.session),
            "DOM.scrollIntoViewIfNeeded",
            json!({"backendNodeId": e.backend}),
        )
        .await;
    let q = quads(cdp, e).await.ok_or_else(|| stale(r, e))?;
    let off = chain_offset(cdp, &e.chain).await?;
    let xs = [q[0], q[2], q[4], q[6]];
    let ys = [q[1], q[3], q[5], q[7]];
    let (x0, x1) = (
        xs.iter().copied().fold(f64::MAX, f64::min),
        xs.iter().copied().fold(f64::MIN, f64::max),
    );
    let (y0, y1) = (
        ys.iter().copied().fold(f64::MAX, f64::min),
        ys.iter().copied().fold(f64::MIN, f64::max),
    );
    Ok((
        x0 + off.0,
        y0 + off.1,
        (x1 - x0).max(1.0),
        (y1 - y0).max(1.0),
    ))
}
