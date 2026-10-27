//! One running profile (browser-control.md §4.3, §6): the browser process, its tabs and the CDP
//! event pump that keeps tab state, logs, dialogs, popups, downloads and viewers current.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use base64::Engine as _;
use oino_browser_protocol::{
    BrowserViewEvent, FrameData, TabLifecycle, TabOwner, ToolEvent, redact_url,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::cdp::{Cdp, CdpError, CdpEvent};
use crate::hub::{Hub, sanitize};
use crate::launch::BrowserProcess;
use crate::tab::{ChildSession, DialogInfo, NETWORK_CAP, NetEntry, Ring, Tab, lock};

/// A finished or running download.
#[derive(Clone, Debug)]
pub(crate) struct Download {
    pub(crate) guid: String,
    pub(crate) tab: String,
    pub(crate) session: Option<String>,
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) bytes: u64,
    pub(crate) reported: bool,
}

/// A running profile.
pub(crate) struct ProfileRt {
    pub(crate) name: String,
    pub(crate) generation: u64,
    pub(crate) process: BrowserProcess,
    pub(crate) cdp: Arc<Cdp>,
    pub(crate) version: String,
    pub(crate) engine: String,
    pub(crate) tabs: Mutex<HashMap<String, Arc<Tab>>>,
    pub(crate) last_used: Mutex<Instant>,
    pub(crate) downloads: Mutex<Vec<Download>>,
    pub(crate) download_done: Notify,
    pub(crate) downloads_tmp: PathBuf,
    pub(crate) downloads_base: Option<PathBuf>,
    pub(crate) dead: AtomicBool,
    pub(crate) hub: Arc<Hub>,
    keeper: Mutex<Option<String>>,
}

impl std::fmt::Debug for ProfileRt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileRt")
            .field("name", &self.name)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// Site permissions denied on every launch (§10.3).
const DENIED: &[&str] = &[
    "geolocation",
    "notifications",
    "camera",
    "microphone",
    "clipboard-read",
    "midi",
    "accelerometer",
    "gyroscope",
    "magnetometer",
    "ambient-light-sensor",
    "idle-detection",
    "display-capture",
    "storage-access",
    "window-management",
    "local-fonts",
    "nfc",
];

impl ProfileRt {
    /// Wraps a launched process: discovery, downloads, default-deny permissions, pump.
    pub(crate) async fn start(
        name: String,
        generation: u64,
        engine: String,
        process: BrowserProcess,
        hub: Arc<Hub>,
        downloads_base: Option<PathBuf>,
        grants: &HashMap<String, HashMap<String, String>>,
    ) -> Result<Arc<Self>, CdpError> {
        let cdp = Arc::clone(&process.cdp);
        let version = cdp
            .call(None, "Browser.getVersion", json!({}))
            .await?
            .get("product")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let downloads_tmp = hub
            .paths
            .root
            .join("downloads")
            .join(".incoming")
            .join(&name);
        let _ = std::fs::create_dir_all(&downloads_tmp);
        let rt = Arc::new(Self {
            name,
            generation,
            process,
            cdp: Arc::clone(&cdp),
            version,
            engine,
            tabs: Mutex::new(HashMap::new()),
            last_used: Mutex::new(Instant::now()),
            downloads: Mutex::new(Vec::new()),
            download_done: Notify::new(),
            downloads_tmp,
            downloads_base,
            dead: AtomicBool::new(false),
            hub,
            keeper: Mutex::new(None),
        });
        let events = cdp.subscribe();
        tokio::spawn(pump(Arc::downgrade(&rt), events));
        cdp.call(None, "Target.setDiscoverTargets", json!({"discover": true}))
            .await?;
        let _ = cdp
            .call(
                None,
                "Browser.setDownloadBehavior",
                json!({"behavior": "allowAndName", "downloadPath": rt.downloads_tmp, "eventsEnabled": true}),
            )
            .await;
        rt.apply_permissions(grants).await;
        if let Ok(targets) = cdp.call(None, "Target.getTargets", json!({})).await {
            let keeper = targets
                .get("targetInfos")
                .and_then(Value::as_array)
                .and_then(|ts| {
                    ts.iter()
                        .find(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                })
                .and_then(|t| t.get("targetId").and_then(Value::as_str))
                .map(str::to_owned);
            *lock(&rt.keeper) = keeper;
        }
        Ok(rt)
    }

    /// Default-deny site permissions, then the user's per-origin grants.
    pub(crate) async fn apply_permissions(
        &self,
        grants: &HashMap<String, HashMap<String, String>>,
    ) {
        for name in DENIED {
            let mut desc = json!({"name": name});
            if *name == "midi" {
                desc["sysex"] = json!(true);
            }
            if let Err(e) = self
                .cdp
                .call(
                    None,
                    "Browser.setPermission",
                    json!({"permission": desc, "setting": "denied"}),
                )
                .await
            {
                tracing::debug!(perm = name, %e, "browser: setPermission");
            }
        }
        let _ = self
            .cdp
            .call(
                None,
                "Browser.setPermission",
                json!({"permission": {"name": "clipboard-write"}, "setting": "granted"}),
            )
            .await;
        for (origin, perms) in grants {
            for (perm, setting) in perms {
                let setting = match setting.as_str() {
                    "allow" => "granted",
                    "block" => "denied",
                    _ => "prompt",
                };
                let r = self
                    .cdp
                    .call(
                        None,
                        "Browser.setPermission",
                        json!({"permission": {"name": perm}, "setting": setting, "origin": origin}),
                    )
                    .await;
                tracing::info!(
                    "[browser-permission] decision={setting} origin={origin} perm={perm} source=user ok={}",
                    r.is_ok()
                );
            }
        }
    }

    pub(crate) fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
    }

    pub(crate) fn tab(&self, id: &str) -> Option<Arc<Tab>> {
        lock(&self.tabs).get(id).cloned()
    }

    pub(crate) fn all_tabs(&self) -> Vec<Arc<Tab>> {
        let mut v: Vec<Arc<Tab>> = lock(&self.tabs).values().cloned().collect();
        v.sort_by_key(|t| t.id.trim_start_matches('t').parse::<u64>().unwrap_or(0));
        v
    }

    fn tab_by_target(&self, target: &str) -> Option<Arc<Tab>> {
        lock(&self.tabs)
            .values()
            .find(|t| t.target_id == target)
            .cloned()
    }

    fn tab_by_session(&self, session: &str) -> Option<Arc<Tab>> {
        lock(&self.tabs)
            .values()
            .find(|t| t.owns_session(session))
            .cloned()
    }

    /// Opens a new page target and registers it.
    pub(crate) async fn open_tab(
        self: &Arc<Self>,
        owner: TabOwner,
        url: &str,
    ) -> Result<Arc<Tab>, CdpError> {
        let reply = self
            .cdp
            .call(
                None,
                "Target.createTarget",
                json!({"url": "about:blank", "background": true}),
            )
            .await?;
        let target = reply
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::Protocol {
                method: "Target.createTarget".into(),
                message: "no targetId".into(),
            })?
            .to_owned();
        let tab = self.attach(&target, owner).await?;
        if url != "about:blank" {
            let _ = self
                .cdp
                .call(Some(&tab.session), "Page.navigate", json!({"url": url}))
                .await;
        }
        Ok(tab)
    }

    /// Attaches to `target` and registers a tab for it.
    pub(crate) async fn attach(
        self: &Arc<Self>,
        target: &str,
        owner: TabOwner,
    ) -> Result<Arc<Tab>, CdpError> {
        let reply = self
            .cdp
            .call(
                None,
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
            )
            .await?;
        let session = reply
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::Protocol {
                method: "Target.attachToTarget".into(),
                message: "no sessionId".into(),
            })?
            .to_owned();
        let owner_session = owner.session().map(str::to_owned);
        let tab = Tab::new(
            self.hub.tab_id(),
            self.name.clone(),
            target.to_owned(),
            session.clone(),
            owner,
        );
        lock(&self.tabs).insert(tab.id.clone(), Arc::clone(&tab));
        let s = Some(session.as_str());
        for (m, p) in [
            ("Page.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("Network.enable", json!({"maxPostDataSize": 0})),
            ("DOM.enable", json!({})),
            ("Page.setLifecycleEventsEnabled", json!({"enabled": true})),
            (
                "Emulation.setFocusEmulationEnabled",
                json!({"enabled": true}),
            ),
            (
                "Emulation.setDeviceMetricsOverride",
                json!({"width": 1280, "height": 800, "deviceScaleFactor": 1, "mobile": false}),
            ),
            (
                "Target.setAutoAttach",
                json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true}),
            ),
        ] {
            if let Err(e) = self.cdp.call(s, m, p).await {
                tracing::debug!(method = m, %e, "browser: tab setup");
            }
        }
        self.hub.emit(
            owner_session.as_deref(),
            BrowserViewEvent::Tabs {
                tabs: self.session_tabs(owner_session.as_deref()),
            },
        );
        Ok(tab)
    }

    /// Tabs of `session` (all of this profile's tabs owned by it).
    pub(crate) fn session_tabs(
        &self,
        session: Option<&str>,
    ) -> Vec<oino_browser_protocol::TabInfo> {
        self.all_tabs()
            .iter()
            .map(|t| t.info())
            .filter(|i| i.owner.session() == session)
            .collect()
    }

    /// Closes a tab.
    pub(crate) async fn close_tab(&self, tab: &Arc<Tab>, reason: &str) {
        lock(&tab.state).closing = true;
        let _ = self
            .cdp
            .call(
                None,
                "Target.closeTarget",
                json!({"targetId": tab.target_id}),
            )
            .await;
        self.forget(tab, reason);
    }

    fn forget(&self, tab: &Arc<Tab>, reason: &str) {
        if lock(&self.tabs).remove(&tab.id).is_none() {
            return;
        }
        let session = {
            let mut s = lock(&tab.state);
            s.lifecycle = TabLifecycle::Closed;
            if let Some((_, tx)) = s.handoff.take() {
                let _ = tx.send(false);
            }
            s.owner.session().map(str::to_owned)
        };
        tab.changed.notify_waiters();
        let closed = BrowserViewEvent::Closed {
            tab: tab.id.clone(),
            reason: reason.to_owned(),
        };
        tab.broadcast_view(&closed);
        self.hub.emit(session.as_deref(), closed);
        if let Some(s) = &session
            && self.hub.current(s).as_deref() == Some(tab.id.as_str())
        {
            self.hub.set_current(s, None);
        }
        self.hub.emit(
            session.as_deref(),
            BrowserViewEvent::Tabs {
                tabs: self.session_tabs(session.as_deref()),
            },
        );
    }

    /// Stops the browser.
    pub(crate) async fn shutdown(&self, reason: &str) {
        self.dead.store(true, Ordering::SeqCst);
        for tab in self.all_tabs() {
            self.forget(&tab, reason);
        }
        self.process.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.downloads_tmp);
    }

    /// Whether anyone is looking at or using this profile.
    pub(crate) fn idle_for(&self) -> Duration {
        let busy = self.all_tabs().iter().any(|t| {
            let visible = !lock(&t.views).is_empty();
            let s = lock(&t.state);
            visible || s.driving || s.handoff.is_some()
        });
        if busy {
            return Duration::ZERO;
        }
        lock(&self.last_used).elapsed()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one step list per operation, clearer kept together"
    )]
    fn page_event(self: &Arc<Self>, tab: &Arc<Tab>, ev: &CdpEvent) {
        let p = &ev.params;
        let is_page = ev.session.as_deref() == Some(tab.session.as_str());
        match ev.method.as_str() {
            "Page.frameNavigated" if is_page => {
                let frame = &p["frame"];
                if frame.get("parentId").is_none() {
                    let url = frame["url"].as_str().unwrap_or_default().to_owned();
                    let mut s = lock(&tab.state);
                    s.url.clone_from(&url);
                    s.refs.clear();
                    s.last_activity = Instant::now();
                    if let Some(c) = s.collecting.as_mut() {
                        c.push(ToolEvent::Navigated {
                            url: redact_url(&url),
                        });
                    }
                    drop(s);
                    self.page_changed(tab);
                }
            }
            "Page.frameStartedLoading" if is_page => {
                if p["frameId"].as_str() == Some(tab.target_id.as_str()) {
                    lock(&tab.state).loading = true;
                    self.page_changed(tab);
                }
            }
            "Page.frameStoppedLoading" if is_page => {
                if p["frameId"].as_str() == Some(tab.target_id.as_str()) {
                    lock(&tab.state).loading = false;
                    self.page_changed(tab);
                    let me = Arc::clone(self);
                    let t = Arc::clone(tab);
                    tokio::spawn(async move { me.refresh_history(&t).await });
                }
            }
            "Page.javascriptDialogOpening" if is_page => {
                let d = DialogInfo {
                    kind: p["type"].as_str().unwrap_or("alert").to_owned(),
                    message: p["message"].as_str().unwrap_or_default().to_owned(),
                    default_prompt: p["defaultPrompt"].as_str().map(str::to_owned),
                };
                tab.note(ToolEvent::Dialog {
                    dialog: d.kind.clone(),
                    message: d.message.clone(),
                });
                let session = {
                    let mut s = lock(&tab.state);
                    s.dialog = Some(d.clone());
                    s.owner.session().map(str::to_owned)
                };
                let e = BrowserViewEvent::Dialog {
                    tab: tab.id.clone(),
                    dialog: d.kind,
                    message: d.message,
                    default_prompt: d.default_prompt,
                };
                tab.broadcast_view(&e);
                self.hub.emit(session.as_deref(), e);
                tab.changed.notify_waiters();
            }
            "Page.javascriptDialogClosed" if is_page => {
                let session = {
                    let mut s = lock(&tab.state);
                    s.dialog = None;
                    s.owner.session().map(str::to_owned)
                };
                let e = BrowserViewEvent::DialogClosed {
                    tab: tab.id.clone(),
                };
                tab.broadcast_view(&e);
                self.hub.emit(session.as_deref(), e);
                tab.changed.notify_waiters();
            }
            "Page.fileChooserOpened" if is_page => {
                if let Some(tx) = lock(&tab.state).file_chooser.take() {
                    let _ = tx.send(p["backendNodeId"].as_i64().unwrap_or(0));
                }
            }
            "Page.screencastFrame" if is_page => self.frame(tab, p),
            "Runtime.consoleAPICalled" => {
                let args: Vec<String> = p["args"]
                    .as_array()
                    .map(|a| a.iter().map(remote_text).collect())
                    .unwrap_or_default();
                tab.log(
                    Ring::Console,
                    format!(
                        "[{}] {}",
                        p["type"].as_str().unwrap_or("log"),
                        args.join(" ")
                    ),
                );
            }
            "Runtime.exceptionThrown" => {
                let d = &p["exceptionDetails"];
                let text = d["exception"]["description"]
                    .as_str()
                    .or_else(|| d["text"].as_str())
                    .unwrap_or("error");
                let first = text.lines().next().unwrap_or(text).to_owned();
                tab.log(Ring::Errors, first);
                let mut s = lock(&tab.state);
                if let Some(c) = s.collecting.as_mut() {
                    if let Some(ToolEvent::Errors { count }) =
                        c.iter_mut().find(|e| matches!(e, ToolEvent::Errors { .. }))
                    {
                        *count += 1;
                    } else {
                        c.push(ToolEvent::Errors { count: 1 });
                    }
                }
            }
            "Network.requestWillBeSent" => {
                let mut s = lock(&tab.state);
                s.log_seq += 1;
                let id = p["requestId"].as_str().unwrap_or_default().to_owned();
                let entry = NetEntry {
                    seq: s.log_seq,
                    id: id.clone(),
                    method: p["request"]["method"].as_str().unwrap_or("GET").to_owned(),
                    url: p["request"]["url"].as_str().unwrap_or_default().to_owned(),
                    kind: p["type"].as_str().unwrap_or_default().to_owned(),
                    started: p["timestamp"].as_f64(),
                    ..NetEntry::default()
                };
                s.network.push_back(entry);
                while s.network.len() > NETWORK_CAP {
                    s.network.pop_front();
                }
                s.inflight.insert(id, Instant::now());
                s.last_network = Instant::now();
            }
            "Network.responseReceived" => {
                let mut s = lock(&tab.state);
                let id = p["requestId"].as_str().unwrap_or_default();
                if let Some(e) = s.network.iter_mut().rev().find(|e| e.id == id) {
                    e.status = p["response"]["status"].as_i64();
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                let mut s = lock(&tab.state);
                let id = p["requestId"].as_str().unwrap_or_default().to_owned();
                if let Some(e) = s.network.iter_mut().rev().find(|e| e.id == id) {
                    e.size = p["encodedDataLength"].as_f64().map(crate::page::px);
                    e.ms = p["timestamp"]
                        .as_f64()
                        .zip(e.started)
                        .map(|(a, b)| (a - b) * 1000.0);
                    e.failed = p["errorText"].as_str().map(str::to_owned);
                }
                s.inflight.remove(&id);
                s.last_network = Instant::now();
                drop(s);
                tab.changed.notify_waiters();
            }
            "Target.attachedToTarget" => {
                let info = &p["targetInfo"];
                let child = ChildSession {
                    session: p["sessionId"].as_str().unwrap_or_default().to_owned(),
                    target_id: info["targetId"].as_str().unwrap_or_default().to_owned(),
                    parent_session: ev.session.clone().unwrap_or_default(),
                    kind: info["type"].as_str().unwrap_or_default().to_owned(),
                };
                let cdp = Arc::clone(&self.cdp);
                let sid = child.session.clone();
                let iframe = child.kind == "iframe";
                lock(&tab.state).children.push(child);
                tokio::spawn(async move {
                    let s = Some(sid.as_str());
                    if iframe {
                        for m in ["Runtime.enable", "DOM.enable", "Page.enable"] {
                            let _ = cdp.call(s, m, json!({})).await;
                        }
                        let _ = cdp
                            .call(
                                s,
                                "Target.setAutoAttach",
                                json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true}),
                            )
                            .await;
                    }
                    let _ = cdp
                        .call(s, "Runtime.runIfWaitingForDebugger", json!({}))
                        .await;
                });
            }
            "Target.detachedFromTarget" => {
                let sid = p["sessionId"].as_str().unwrap_or_default();
                lock(&tab.state).children.retain(|c| c.session != sid);
            }
            "Inspector.targetCrashed" if is_page => {
                let url = {
                    let mut s = lock(&tab.state);
                    let ok = s
                        .crash_reload_at
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(60));
                    if ok {
                        s.crash_reload_at = Some(Instant::now());
                    }
                    ok.then(|| s.url.clone())
                };
                tab.log(Ring::Errors, "the page crashed".into());
                if let Some(url) = url {
                    let cdp = Arc::clone(&self.cdp);
                    let sid = tab.session.clone();
                    tokio::spawn(async move {
                        let _ = cdp
                            .call(Some(&sid), "Page.navigate", json!({"url": url}))
                            .await;
                    });
                }
            }
            _ => {}
        }
    }

    async fn refresh_history(&self, tab: &Arc<Tab>) {
        if let Ok(h) = self
            .cdp
            .call(Some(&tab.session), "Page.getNavigationHistory", json!({}))
            .await
        {
            let idx = h["currentIndex"].as_i64().unwrap_or(0);
            let n = i64::try_from(h["entries"].as_array().map_or(0, Vec::len)).unwrap_or(i64::MAX);
            lock(&tab.state).history = (idx > 0, idx + 1 < n);
            self.page_changed(tab);
        }
    }

    pub(crate) fn page_changed(&self, tab: &Arc<Tab>) {
        let (e, session) = {
            let s = lock(&tab.state);
            (
                BrowserViewEvent::Page {
                    tab: tab.id.clone(),
                    url: s.url.clone(),
                    title: s.title.clone(),
                    loading: s.loading,
                    can_back: s.history.0,
                    can_forward: s.history.1,
                },
                s.owner.session().map(str::to_owned),
            )
        };
        tab.changed.notify_waiters();
        tab.broadcast_view(&e);
        self.hub.emit(session.as_deref(), e);
    }

    fn frame(&self, tab: &Arc<Tab>, p: &Value) {
        let ack = p["sessionId"].as_i64().unwrap_or(0);
        self.cdp.fire(
            Some(&tab.session),
            "Page.screencastFrameAck",
            &json!({"sessionId": ack}),
        );
        let Some(data) = p["data"]
            .as_str()
            .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
        else {
            return;
        };
        let meta = &p["metadata"];
        let data = FrameData(Arc::new(data));
        let (w, h) = (
            meta["deviceWidth"].as_f64().unwrap_or(1280.0),
            meta["deviceHeight"].as_f64().unwrap_or(800.0),
        );
        push_frame(tab, &data, w, h);
    }

    fn browser_event(self: &Arc<Self>, ev: &CdpEvent) {
        let p = &ev.params;
        match ev.method.as_str() {
            "Target.targetCreated" => {
                let info = &p["targetInfo"];
                if info["type"].as_str() != Some("page") {
                    return;
                }
                let Some(opener) = info["openerId"].as_str() else {
                    return;
                };
                let Some(parent) = self.tab_by_target(opener) else {
                    return;
                };
                let target = info["targetId"].as_str().unwrap_or_default().to_owned();
                if self.tab_by_target(&target).is_some() {
                    return;
                }
                let owner = lock(&parent.state).owner.clone();
                let me = Arc::clone(self);
                tokio::spawn(async move {
                    if let Ok(tab) = me.attach(&target, owner).await {
                        parent.note(ToolEvent::Popup {
                            tab: tab.id.clone(),
                        });
                        parent.changed.notify_waiters();
                    }
                });
            }
            "Target.targetInfoChanged" => {
                let info = &p["targetInfo"];
                if let Some(tab) = info["targetId"]
                    .as_str()
                    .and_then(|t| self.tab_by_target(t))
                {
                    {
                        let mut s = lock(&tab.state);
                        if let Some(u) = info["url"].as_str() {
                            u.clone_into(&mut s.url);
                        }
                        if let Some(t) = info["title"].as_str() {
                            s.title = crate::page::clean_title(t, &s.url);
                        }
                    }
                    self.page_changed(&tab);
                }
            }
            "Target.targetDestroyed" => {
                if let Some(tab) = p["targetId"].as_str().and_then(|t| self.tab_by_target(t)) {
                    let closing = lock(&tab.state).closing;
                    self.forget(&tab, if closing { "agent" } else { "user" });
                }
                if p["targetId"].as_str() == lock(&self.keeper).as_deref() {
                    *lock(&self.keeper) = None;
                }
            }
            "Browser.downloadWillBegin" => {
                let frame = p["frameId"].as_str().unwrap_or_default();
                let tab = self.tab_by_target(frame);
                let name = sanitize(p["suggestedFilename"].as_str().unwrap_or("download"));
                if let Some(t) = &tab {
                    t.note(ToolEvent::Download { name: name.clone() });
                    lock(&t.state).downloads_active += 1;
                }
                let session = tab
                    .as_ref()
                    .and_then(|t| lock(&t.state).owner.session().map(str::to_owned));
                let tab_id = tab.as_ref().map(|t| t.id.clone()).unwrap_or_default();
                self.hub.emit(
                    session.as_deref(),
                    BrowserViewEvent::Download {
                        tab: tab_id.clone(),
                        name: name.clone(),
                        state: "started".into(),
                    },
                );
                lock(&self.downloads).push(Download {
                    guid: p["guid"].as_str().unwrap_or_default().to_owned(),
                    tab: tab_id,
                    session,
                    name,
                    state: "inProgress".into(),
                    path: None,
                    bytes: 0,
                    reported: false,
                });
            }
            "Browser.downloadProgress" => self.download_progress(p),
            _ => {}
        }
    }

    fn download_progress(&self, p: &Value) {
        let guid = p["guid"].as_str().unwrap_or_default();
        let state = p["state"].as_str().unwrap_or_default();
        if state == "inProgress" {
            return;
        }
        let done = {
            let mut list = lock(&self.downloads);
            let Some(d) = list.iter_mut().find(|d| d.guid == guid) else {
                return;
            };
            state.clone_into(&mut d.state);
            d.bytes = p["receivedBytes"].as_u64().unwrap_or(0);
            if state == "completed" {
                let dir = self.hub.paths.downloads(
                    self.downloads_base.as_ref(),
                    d.session.as_deref().unwrap_or("none"),
                );
                let _ = std::fs::create_dir_all(&dir);
                let dest = unique(&dir, &d.name);
                let src = self.downloads_tmp.join(guid);
                if std::fs::rename(&src, &dest).is_err() {
                    let _ = std::fs::copy(&src, &dest);
                    let _ = std::fs::remove_file(&src);
                }
                d.path = Some(dest);
            }
            d.clone()
        };
        if let Some(t) = self.tab(&done.tab) {
            let mut s = lock(&t.state);
            s.downloads_active = s.downloads_active.saturating_sub(1);
        }
        self.hub.emit(
            done.session.as_deref(),
            BrowserViewEvent::Download {
                tab: done.tab.clone(),
                name: done.name.clone(),
                state: state.to_owned(),
            },
        );
        self.download_done.notify_waiters();
    }
}

/// Fans a frame out to a tab's viewers (≤ 2 un-acked frames each).
pub(crate) fn push_frame(tab: &Tab, data: &FrameData, w: f64, h: f64) {
    lock(&tab.views).retain_mut(|v| {
        if v.unacked >= 2 {
            return !v.tx.is_closed();
        }
        v.last_seq += 1;
        let e = BrowserViewEvent::Frame {
            view_id: v.id,
            tab: tab.id.clone(),
            seq: v.last_seq,
            css_width: w,
            css_height: h,
            jpeg: data.clone(),
        };
        match v.tx.try_send(e) {
            Ok(()) => {
                v.unacked += 1;
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        }
    });
}

fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name
        .rsplit_once('.')
        .map_or((name, String::new()), |(s, e)| (s, format!(".{e}")));
    (1..1000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn remote_text(v: &Value) -> String {
    match v.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => v["description"]
            .as_str()
            .or_else(|| v["type"].as_str())
            .unwrap_or_default()
            .to_owned(),
    }
}

async fn pump(rt: Weak<ProfileRt>, mut events: tokio::sync::broadcast::Receiver<Arc<CdpEvent>>) {
    loop {
        let ev = match events.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(n, "browser: cdp events lagged");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let Some(rt) = rt.upgrade() else { return };
        if ev.method == "Oino.disconnected" {
            rt.dead.store(true, Ordering::SeqCst);
            for tab in rt.all_tabs() {
                rt.forget(&tab, "crash");
            }
            return;
        }
        match ev.session.as_deref() {
            None => rt.browser_event(&ev),
            Some(s) => {
                if let Some(tab) = rt.tab_by_session(s) {
                    rt.page_event(&tab, &ev);
                }
            }
        }
    }
}
