//! The `"browser"` host service (browser.md §7, WP-B7): the agent's 19 `browser_*` tools.
//!
//! The `unipi.browser` extension calls `host-service.call("browser", op, BrowserRequest)`;
//! this service runs it on [`unipi_browser::BrowserService`] — snapshots with refs,
//! actionability, approvals, turn-tail shots — in **attached mode**: the service never
//! launches a browser of its own. It asks this host's [`Manager`] for the profile's
//! browser (the same CloakBrowser / Chrome the app's Browser pane and the phone sheet
//! watch) and connects to its gated DevTools endpoint, so the agent's commands pass the
//! take-over check and show up as "Agent is driving" in every viewer.
//!
//! Consent (UNI-301 semantics, browser.md §7): agents on this PC count as the PC's own
//! app (`LOCAL_DEVICE_ID`) — on unless `config.json` says `browser.enabled: false`
//! explicitly (`unipi-host browser disable`). Phones are not involved: their viewer ops
//! still need `enabled: true` plus the device allow-list. `settings.json`
//! `browser.enabled: false` (the oino setting) also switches the tools off.
//!
//! Scope: each call carries the caller's [`CallContext`]; tabs belong to the root
//! conversation session (nested agents resolve to their root through the wasm hub) in
//! the session's cwd.

use std::path::Path;
use std::sync::{Arc, Weak};

use futures_util::future::BoxFuture;
use oino_extension_api::host_service::HostService;
use oino_extension_api::{CallContext, CancellationToken, HostError, RawJson};
use oino_wasm_host::LocalServices;
use serde_json::Value;
use unipi_browser::protocol::{BrowserResult, BrowserScope, UnavailableReason};
use unipi_browser::{
    Attach, AttachError, Attached, AttachedDisplay, BrowserHost, BrowserOptions, BrowserRequest,
    BrowserService, BrowserSettings,
};

use super::{would_use, BrowserConfig, Engine, Manager, Placement};
use crate::data_dir::HostPaths;
use crate::protocol::LOCAL_DEVICE_ID;

/// What the model reads when the owner switched the browser off.
pub const DISABLED_MESSAGE: &str =
    "the browser is off on this PC (`unipi-host browser enable` turns it on)";

/// How often a pending handoff checks whether the user gave the browser back.
const HANDOFF_POLL: std::time::Duration = std::time::Duration::from_millis(400);

/// Is the browser on for this PC's own agents? (`config.json`, UNI-301 rules.)
pub fn enabled_for_agents(config: &BrowserConfig) -> bool {
    config.is_enabled_for(LOCAL_DEVICE_ID)
}

fn display_of(placement: Placement) -> AttachedDisplay {
    match placement {
        Placement::Hidden => AttachedDisplay::Hidden,
        Placement::Window => AttachedDisplay::Window,
        Placement::Headless => AttachedDisplay::Headless,
    }
}

/// A failed [`Manager::start`] as the tool-facing reason + message.
fn attach_error(e: unipi_proto::AppErrorDto) -> AttachError {
    let reason = e
        .detail
        .as_ref()
        .and_then(|d| d.get("reason"))
        .and_then(Value::as_str)
        .map_or("launch_failed", |r| match r {
            "not_found" => "no_engine",
            other => other,
        });
    AttachError {
        reason: reason.to_owned(),
        message: e.message.as_str().to_owned(),
    }
}

/// [`Attach`] backed by this host's browser [`Manager`].
pub struct ManagerAttach {
    paths: HostPaths,
}

impl ManagerAttach {
    pub fn new(paths: HostPaths) -> Self {
        Self { paths }
    }
}

impl Attach for ManagerAttach {
    fn attach(&self, profile: &str) -> BoxFuture<'_, Result<Attached, AttachError>> {
        let profile = profile.to_owned();
        Box::pin(async move {
            let config = BrowserConfig::load(&self.paths.config()).map_err(|e| AttachError {
                reason: "launch_failed".into(),
                message: format!("could not read the host config: {e}"),
            })?;
            if !enabled_for_agents(&config) {
                return Err(AttachError {
                    reason: "disabled".into(),
                    message: DISABLED_MESSAGE.into(),
                });
            }
            let manager = Manager::for_paths(&self.paths);
            let (_instance, running) = manager
                .start(&profile, &config, None)
                .await
                .map_err(attach_error)?;
            let ws_url = running.gate_ws().ok_or_else(|| AttachError {
                reason: "launch_failed".into(),
                message: "the browser runs but its agent endpoint did not open".into(),
            })?;
            Ok(Attached {
                ws_url,
                engine: running.engine.as_str().to_owned(),
                display: display_of(running.placement),
            })
        })
    }
}

/// The `"browser"` host service.
pub struct BrowserHostService {
    svc: BrowserService,
    paths: HostPaths,
    /// Resolves nested agents to their root session (§6.1).
    hub: Option<Weak<LocalServices>>,
}

impl BrowserHostService {
    /// The service for this host: tools state under `<agent_dir>/browser`, settings from
    /// `<agent_dir>/settings.json` (`browser.*`), browsers from the host's [`Manager`].
    pub fn new(paths: HostPaths, agent_dir: &Path, hub: Option<&Arc<LocalServices>>) -> Self {
        let mut settings = BrowserSettings::load(agent_dir);
        // The gate humanizes every agent input already (UNI-293); doing it twice would
        // only slow the agent down.
        settings.humanize = unipi_browser::config::Humanize::Off;
        let svc = BrowserService::new(BrowserOptions {
            root: agent_dir.join("browser"),
            settings,
            ephemeral: false,
            attach: Some(Arc::new(ManagerAttach::new(paths.clone()))),
        });
        Self {
            svc,
            paths,
            hub: hub.map(Arc::downgrade),
        }
    }

    /// The underlying service (tests, shutdown).
    pub fn service(&self) -> &BrowserService {
        &self.svc
    }

    fn scope(&self, session_id: &str, cwd: &Path) -> BrowserScope {
        let root = self
            .hub
            .as_ref()
            .and_then(Weak::upgrade)
            .map_or_else(|| session_id.to_owned(), |hub| hub.root_session(session_id));
        BrowserScope::session(root, cwd.display().to_string())
    }

    fn config(&self) -> BrowserConfig {
        BrowserConfig::load(&self.paths.config()).unwrap_or_default()
    }

    /// `status`: the service's status (`available`, session tabs, profiles) with this
    /// host's consent and engine facts.
    async fn status(&self, scope: &BrowserScope) -> Value {
        let config = self.config();
        let mut st = self.svc.status(Some(scope));
        if !enabled_for_agents(&config) {
            st.enabled = false;
            st.available = false;
            st.reason = Some(UnavailableReason::Disabled);
        }
        let running = Manager::for_paths(&self.paths).current().await;
        let (info, engine, stealth) = match &running {
            Some(r) => (Some(r.info.clone()), r.engine, r.stealth),
            None => {
                let (info, engine, stealth, _) = would_use(&config);
                (info, engine, stealth)
            }
        };
        st.engine = Some(engine.as_str().to_owned());
        st.stealth = stealth;
        if let Some(info) = info {
            st.product = Some(info.name);
            st.executable = Some(info.path);
            if st.version.is_none() {
                st.version = info.version;
            }
        }
        if let Some(r) = &running {
            st.display = r.placement.as_str().to_owned();
            st.stealth_note = r.stealth_note();
        }
        if st.stealth_note.is_none() && !stealth && engine == Engine::Chrome {
            st.stealth_note = Some("Not stealthy: a regular Chrome-family browser".into());
        }
        serde_json::to_value(&st).unwrap_or(Value::Null)
    }

    /// `browser_handoff` in attached mode: the agent hands the browser to the user, so the
    /// viewers' gate flips to "You have control · the agent waits"; the user's **Give
    /// back** (or the gate's idle expiry) ends the handoff as Done. A handoff that ends on
    /// its own (timeout, cancel) gives the gate back too.
    async fn handoff(
        &self,
        scope: BrowserScope,
        request: BrowserRequest,
        cancel: CancellationToken,
    ) -> BrowserResult {
        let profile = self
            .svc
            .status(Some(&scope))
            .session_profile
            .unwrap_or_else(|| super::profiles::DEFAULT.to_owned());
        let gate = Manager::for_paths(&self.paths).gate(&profile);
        gate.set_takeover(true);
        gate.note_user_input();
        let watcher = {
            let (gate, svc, profile) = (Arc::clone(&gate), self.svc.clone(), profile.clone());
            tokio::spawn(async move {
                // Give the handoff a moment to register before watching for the give-back.
                tokio::time::sleep(HANDOFF_POLL).await;
                while gate.taken_over() {
                    tokio::time::sleep(HANDOFF_POLL).await;
                }
                let ended = svc.end_handoffs(&profile);
                tracing::info!(op = "browser_handoff", ended, "given back from the viewer");
            })
        };
        let result = self.svc.execute(scope, request, cancel).await;
        watcher.abort();
        if gate.taken_over() {
            gate.set_takeover(false);
        }
        result
    }

    async fn run(
        &self,
        scope: BrowserScope,
        op: &str,
        payload: RawJson,
        cancel: CancellationToken,
    ) -> Result<RawJson, HostError> {
        if op == "status" {
            return Ok(RawJson::from_json_value(&self.status(&scope).await));
        }
        let result = if enabled_for_agents(&self.config()) {
            let mut value = payload.to_value();
            if let Some(obj) = value.as_object_mut() {
                // The op is the call's; the payload repeats it (`{"op": …, …args}`).
                obj.insert("op".into(), Value::String(op.to_owned()));
            }
            let request: BrowserRequest = serde_json::from_value(value)
                .map_err(|e| HostError::InvalidArgument(format!("browser {op}: {e}")))?;
            if op == "handoff" {
                self.handoff(scope, request, cancel).await
            } else {
                self.svc.execute(scope, request, cancel).await
            }
        } else {
            BrowserResult::unavailable("disabled", DISABLED_MESSAGE)
        };
        let value = serde_json::to_value(&result)
            .map_err(|e| HostError::Failed(format!("browser {op}: {e}")))?;
        Ok(RawJson::from_json_value(&value))
    }
}

impl HostService for BrowserHostService {
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<RawJson, HostError>> {
        // No caller context (a direct in-process call): one shared host scope.
        let scope = BrowserScope::session("host", "");
        let op = op.to_owned();
        Box::pin(async move { self.run(scope, &op, payload, cancel).await })
    }

    fn call_in(
        &self,
        cx: &CallContext,
        op: &str,
        payload: RawJson,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<RawJson, HostError>> {
        let scope = self.scope(&cx.session_id, &cx.cwd);
        let op = op.to_owned();
        Box::pin(async move { self.run(scope, &op, payload, cancel).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cx(session: &str, cwd: &str) -> CallContext {
        CallContext {
            session_id: session.into(),
            mode: oino_extension_api::Mode::Rpc,
            has_ui: true,
            cwd: cwd.into(),
            is_child: false,
            child_role: None,
            kind: oino_extension_api::CallKind::Tool,
            tool_call_id: None,
        }
    }

    fn service(dir: &Path, config: Option<Value>) -> BrowserHostService {
        let host = dir.join("host");
        std::fs::create_dir_all(&host).unwrap();
        if let Some(config) = config {
            std::fs::write(host.join("config.json"), config.to_string()).unwrap();
        }
        BrowserHostService::new(HostPaths::from_dir(&host), dir, None)
    }

    async fn call(svc: &BrowserHostService, op: &str, payload: Value) -> Value {
        svc.call_in(
            &cx("s1", "/tmp/w"),
            op,
            RawJson::from_json_value(&payload),
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .to_value()
    }

    #[tokio::test]
    async fn on_by_default_for_this_pcs_agents() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), None);
        let st = call(&svc, "status", json!({})).await;
        assert_eq!(st["available"], json!(true), "{st}");
        assert_eq!(st["enabled"], json!(true));
        assert_eq!(st["sessionProfile"], json!("default"));
        assert!(st["engine"].is_string(), "{st}");
        assert!(svc.service().is_attached());
        // `enabled: true` written by the owner keeps it on too.
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), Some(json!({"browser": {"enabled": true}})));
        assert_eq!(
            call(&svc, "status", json!({})).await["available"],
            json!(true)
        );
    }

    #[tokio::test]
    async fn explicit_off_switches_the_tools_off() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), Some(json!({"browser": {"enabled": false}})));
        let st = call(&svc, "status", json!({})).await;
        assert_eq!(st["available"], json!(false));
        assert_eq!(st["reason"], json!("disabled"));
        // Commands answer `backend_unavailable` without starting anything.
        let r = call(
            &svc,
            "navigate",
            json!({"op": "navigate", "url": "about:blank"}),
        )
        .await;
        assert_eq!(r["isError"], json!(true), "{r}");
        assert!(
            r["text"].as_str().unwrap().contains("off on this PC"),
            "{r}"
        );
        assert!(Manager::for_paths(&svc.paths).current().await.is_none());
    }

    #[tokio::test]
    async fn oino_setting_off_switches_the_tools_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            json!({"browser": {"enabled": false}}).to_string(),
        )
        .unwrap();
        let svc = service(dir.path(), None);
        let st = call(&svc, "status", json!({})).await;
        assert_eq!(st["available"], json!(false));
    }

    #[tokio::test]
    async fn bad_payloads_are_invalid_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), None);
        let err = svc
            .call_in(
                &cx("s1", "/tmp/w"),
                "frobnicate",
                RawJson::from_json_value(&json!({})),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HostError::InvalidArgument(_)), "{err:?}");
    }

    #[tokio::test]
    async fn host_facing_ops_need_no_browser() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), None);
        for op in ["turn_ended", "close_agent_tabs"] {
            let r = call(&svc, op, json!({"op": op})).await;
            assert_eq!(r["isError"], json!(false), "{op}: {r}");
        }
        assert!(Manager::for_paths(&svc.paths).current().await.is_none());
    }

    #[tokio::test]
    async fn scope_is_the_callers_session_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), None);
        let scope = svc.scope("child-7", Path::new("/work/a"));
        assert_eq!(scope.session_id, "child-7");
        assert_eq!(scope.workspace, "/work/a");
        // With a hub, nested agents resolve to their root session.
        let hub = Arc::new(LocalServices::new(dir.path()));
        let svc = BrowserHostService::new(
            HostPaths::from_dir(dir.path().join("host")),
            dir.path(),
            Some(&hub),
        );
        assert_eq!(svc.scope("root", Path::new("/w")).session_id, "root");
    }

    #[tokio::test]
    async fn attach_refuses_when_off() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("host");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(
            host.join("config.json"),
            json!({"browser": {"enabled": false}}).to_string(),
        )
        .unwrap();
        let attach = ManagerAttach::new(HostPaths::from_dir(&host));
        let err = attach.attach("default").await.unwrap_err();
        assert_eq!(err.reason, "disabled");
    }

    /// A one-page HTTP server on 127.0.0.1 (any path → `body`).
    async fn serve_page(body: &'static str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        port
    }

    /// End to end on a real browser (`UNIPI_TEST_BROWSER=1`, CloakBrowser or Chrome on
    /// this machine): the agent's navigate + snapshot run in the Manager's browser — the
    /// one the app's pane shows — through the gated endpoint.
    #[tokio::test(flavor = "multi_thread")]
    async fn real_browser_agent_shares_the_viewers_browser() {
        if std::env::var_os("UNIPI_TEST_BROWSER").is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), None);
        let port = serve_page("<title>Probe</title><h1>Hello from the probe</h1>").await;
        let url = format!("http://127.0.0.1:{port}/");
        let r = call(&svc, "navigate", json!({"op": "navigate", "url": url})).await;
        assert_eq!(r["isError"], json!(false), "{r}");
        let r = call(&svc, "snapshot", json!({"op": "snapshot"})).await;
        assert!(
            r["text"].as_str().unwrap().contains("Hello from the probe"),
            "{r}"
        );
        // Same browser as the viewers: the Manager's default profile lists the agent's tab.
        let running = Manager::for_paths(&svc.paths).current().await.unwrap();
        assert!(
            running.tabs().await.iter().any(|t| t.url == url),
            "{:?}",
            running.tabs().await
        );
        let st = call(&svc, "status", json!({})).await;
        assert_eq!(st["tabs"].as_array().map(Vec::len), Some(1), "{st}");
        // browser_handoff: the viewers' gate shows "You have control"; Give back ends it.
        let gate = Manager::for_paths(&svc.paths).gate("default");
        let give_back = async {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            assert!(gate.taken_over(), "handoff hands the browser to the user");
            gate.set_takeover(false);
        };
        let handoff = call(
            &svc,
            "handoff",
            json!({"op": "handoff", "message": "Check the page", "timeout_s": 30}),
        );
        let (r, ()) = tokio::join!(handoff, give_back);
        assert!(
            r["text"].as_str().unwrap().starts_with("Handoff done"),
            "{r}"
        );
        assert!(!gate.taken_over());
        svc.service().shutdown().await;
        // The service closed its own tab; the host's browser lives on.
        assert!(running.alive());
        assert!(!running.tabs().await.iter().any(|t| t.url == url));
        Manager::for_paths(&svc.paths).stop("default", "test").await;
    }

    #[test]
    fn placements_map_to_status_names() {
        assert_eq!(display_of(Placement::Hidden).as_str(), "hidden");
        assert_eq!(display_of(Placement::Window).as_str(), "window");
        assert_eq!(display_of(Placement::Headless).as_str(), "headless");
    }
}
