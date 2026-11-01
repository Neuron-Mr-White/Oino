//! P0-06 acceptance: the host owns phone push — the session tap's
//! `notify::on_session_event` flips the hub card's `needs_you` on dialogs and
//! sends the old-unipi push shape over the host's own channels: `unipi1:`
//! E2E envelope on the ntfy topic (with the content-free
//! `unipi://s/<host>/<session>` deep link on `click`), UnifiedPush endpoint
//! fan-out, all gated by the `unipi.notify` extension's event matrix.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_core::ui::{DismissReason, UiRequest};
use oino_models::ModelRegistry;
use unipi_host::data_dir::HostPaths;
use unipi_host::notify;
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};

/// Env vars are process-global: every test that touches the notify config or
/// endpoints path runs under this lock and restores on exit.
static ENV: Mutex<()> = Mutex::new(());

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, std::ffi::OsString)]) -> Self {
        let lock = ENV
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (key, value) in vars {
            std::env::set_var(key, value);
        }
        Self { _lock: lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(notify::UNIPI_NOTIFY_CONFIG_ENV);
        std::env::remove_var(notify::UNIPI_NOTIFY_ENDPOINTS_ENV);
    }
}

/// A local ntfy stand-in: accepts JSON publishes, records every raw request,
/// always answers `200`. Runs until the process ends (daemon task).
async fn fake_ntfy() -> (String, Arc<Mutex<Vec<Vec<u8>>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorder = Arc::clone(&recorder);
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 64 * 1024];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                buf.truncate(n);
                recorder.lock().unwrap().push(buf.clone());
                let body = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), seen)
}

struct Fixture {
    _dir: tempfile::TempDir,
    host_data: PathBuf,
    settings: PathBuf,
    endpoints: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            host_data: dir.path().join("host"),
            settings: dir.path().join("settings.json"),
            endpoints: dir.path().join("app-endpoints.json"),
            _dir: dir,
        };
        std::fs::create_dir_all(&fixture.host_data).unwrap();
        std::fs::write(&fixture.endpoints, r#"{"endpoints": []}"#).unwrap();
        fixture
    }

    /// Owner ruling 2026-11-02: writes a paired phone into `devices.json`, so
    /// the default-on pushes (`input_needed`/`agent_settled`) fire even with
    /// an empty explicit matrix.
    fn pair_phone(&self) {
        std::fs::write(
            self.host_data.join("devices.json"),
            r#"[{"id":"ph1","name":"Pixel","paired_at":1}]"#,
        )
        .unwrap();
    }

    /// Writes the notify extension's settings block: ntfy on (pointing at
    /// `server`), plus `events` toggles.
    fn configure(&self, server: &str, events: &str) {
        std::fs::write(
            &self.settings,
            format!(
                r#"{{"extensionSettings":{{"notify":{{
                    "ntfy":{{"enabled":true,"serverUrl":"{server}","topic":"oino-test"}},
                    "defaultPlatforms":["ntfy"],
                    "events":{events}
                }}}}}}"#
            ),
        )
        .unwrap();
    }

    fn paths(&self) -> HostPaths {
        HostPaths::from_dir(&self.host_data)
    }

    /// A hub over the fixture (the `sessions_hub.rs` pattern: faux provider).
    fn hub(&self) -> Arc<SessionHub> {
        let provider =
            FauxProvider::new((0..8).map(|_| FauxResponse::Text("done".into())).collect());
        let dir = self._dir.path();
        SessionHub::create(HostHubOptions {
            cwd: dir.to_path_buf(),
            agent_dir: dir.join("agent"),
            docs_root: None,
            session_dir: Some(dir.join("sessions")),
            mode: ExtensionMode::Desktop,
            has_ui: false,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..ResourceFlags::default()
            },
            loader: Arc::new(StaticLoader(vec![
                Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>
            ])),
            models: {
                let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
                models.set_runtime_api_key("faux", "test-key");
                Some(models)
            },
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            services_cap: None,
            host_data_dir: Some(self.host_data.clone()),
        })
        .unwrap()
    }
}

async fn create(hub: &Arc<SessionHub>) -> HubSession {
    let session = hub.create_session(None, None).await.unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
    session
}

/// Drives one faux turn so core writes the session file (the card's source).
async fn prime(hub: &Arc<SessionHub>, session: &HubSession) {
    let _ = hub;
    let _ = session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "hi".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await;
}

fn dialog(id: &str, method: &str, params: serde_json::Value) -> ClientEvent {
    let mut request = serde_json::from_value::<UiRequest>(serde_json::json!({
        "type": "extension_ui_request", "id": id, "method": method,
    }))
    .unwrap();
    request.params = params.as_object().unwrap().clone();
    ClientEvent::Ui(request)
}

fn dismiss(id: &str) -> ClientEvent {
    ClientEvent::Oino(OinoEvent::ExtensionUiDismiss {
        id: id.to_owned(),
        reason: DismissReason::Answered,
    })
}

/// Decrypts one `unipi1:` envelope with the key file under `paths` (AES-256-GCM,
/// `iv(12) || ciphertext || tag`).
fn decrypt(paths: &HostPaths, envelope: &str) -> serde_json::Value {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::Aes256Gcm;
    use base64::Engine;
    let key: [u8; 32] = std::fs::read(paths.notify_key())
        .expect("notify key")
        .as_slice()
        .try_into()
        .expect("32-byte key");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(envelope.strip_prefix("unipi1:").expect("envelope prefix"))
        .expect("base64");
    let cipher = Aes256Gcm::new((&key).into());
    let iv: [u8; 12] = bytes[..12].try_into().expect("iv");
    let plain = cipher.decrypt((&iv).into(), &bytes[12..]).expect("decrypt");
    serde_json::from_slice(&plain).expect("payload json")
}

/// One POST the fake server saw, parsed as JSON (`None` while nothing arrived).
fn first_json(seen: &Arc<Mutex<Vec<Vec<u8>>>>) -> Option<serde_json::Value> {
    let requests = seen.lock().unwrap();
    requests.first().map(|raw| {
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("http header terminator");
        serde_json::from_slice(&raw[split + 4..]).expect("json body")
    })
}

async fn wait_for_request(seen: &Arc<Mutex<Vec<Vec<u8>>>>, count: usize) {
    for _ in 0..100 {
        if seen.lock().unwrap().len() >= count {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no push arrived (seen {})", seen.lock().unwrap().len());
}

/// Waits (bounded) until the hub lists the session's card (core writes the
/// session file on first activity).
async fn wait_for_card(hub: &Arc<SessionHub>, sid: &str) -> unipi_host::sessions::SessionCard {
    for _ in 0..100 {
        if let Some(card) = hub.card(sid).await.unwrap() {
            return card;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("card never appeared for {sid}");
}

/// Dialogs: `needs_you` flips on the card, and — with a paired phone and no
/// user configuration — the default-on `input_needed` push leaves in the full
/// old-unipi shape: `unipi1:` envelope (decryptable with the host's key,
/// carrying `kind`/`host`/`session`/`options`) plus the `unipi://s/…` `click`.
#[tokio::test]
async fn dialog_open_flips_needs_you_and_pushes_e2e_with_deep_link() {
    let (server, seen) = fake_ntfy().await;
    let fixture = Fixture::new();
    fixture.pair_phone();
    // No event matrix at all: the owner ruling's default-on applies.
    fixture.configure(&server, "{}");
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();
    let paths = fixture.paths();

    notify::on_session_event(
        &hub,
        &paths,
        &dialog(
            "d-1",
            "select",
            serde_json::json!({"title": "Allow bash?", "options": ["Allow", "Deny"]}),
        ),
        &sid,
    )
    .await;

    // The card needs the user.
    assert!(wait_for_card(&hub, &sid).await.needs_you);
    // The push hit the ntfy topic with the deep link on `click`.
    wait_for_request(&seen, 1).await;
    let body = first_json(&seen).unwrap();
    assert_eq!(body["topic"], "oino-test");
    assert_eq!(body["title"], "UniPi");
    let click = body["click"].as_str().unwrap();
    let host = notify::hostname_for_route();
    assert_eq!(click, notify::session_deep_link(&host, &sid));
    // The envelope decrypts to the phone's NotifyPayload.
    let payload = decrypt(&paths, body["message"].as_str().unwrap());
    assert_eq!(payload["kind"], "input_needed");
    assert_eq!(payload["session"], sid.as_str());
    assert_eq!(payload["host"], host);
    assert_eq!(payload["title"], "Oino — Input Needed");
    assert_eq!(payload["body"], "Waiting for your input: Allow bash?");
    assert_eq!(payload["options"], serde_json::json!(["Allow", "Deny"]));

    // Closing the dialog clears the flag; nothing more is sent.
    notify::on_session_event(&hub, &paths, &dismiss("d-1"), &sid).await;
    assert!(!hub.card(&sid).await.unwrap().unwrap().needs_you);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// No phone paired: the default-on matrix sends nothing, but the card's
/// `needs_you` (UI state, not push) still flips.
#[tokio::test]
async fn unpaired_phone_sends_nothing_but_needs_you_still_flips() {
    let (server, seen) = fake_ntfy().await;
    let fixture = Fixture::new(); // no devices, no endpoints
    fixture.configure(&server, "{}");
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();

    notify::on_session_event(
        &hub,
        &fixture.paths(),
        &dialog("d-1", "input", serde_json::json!({"title": "Name?"})),
        &sid,
    )
    .await;
    assert!(wait_for_card(&hub, &sid).await.needs_you);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(seen.lock().unwrap().len(), 0, "no phone paired: no push");
}

/// Rapid successive dialogs de-duplicate to one push (unipi `PROMPT_DEDUP_MS`),
/// and `needs_you` only clears when the *last* dialog closes.
#[tokio::test]
async fn overlapping_dialogs_dedup_pushes_and_keep_needs_you_until_the_last_closes() {
    let (server, seen) = fake_ntfy().await;
    let fixture = Fixture::new();
    fixture.pair_phone();
    fixture.configure(&server, r#"{"input_needed":{"enabled":true}}"#);
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();
    let paths = fixture.paths();

    let open = |id: &str| dialog(id, "confirm", serde_json::json!({"title": "T"}));
    notify::on_session_event(&hub, &paths, &open("d-1"), &sid).await;
    notify::on_session_event(&hub, &paths, &open("d-2"), &sid).await;
    // One push for the two rapid opens (the 2 s de-dup).
    wait_for_request(&seen, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(seen.lock().unwrap().len(), 1, "rapid dialogs de-duplicate");
    assert!(wait_for_card(&hub, &sid).await.needs_you);

    notify::on_session_event(&hub, &paths, &dismiss("d-1"), &sid).await;
    assert!(
        wait_for_card(&hub, &sid).await.needs_you,
        "one dialog still open"
    );
    notify::on_session_event(&hub, &paths, &dismiss("d-2"), &sid).await;
    assert!(!wait_for_card(&hub, &sid).await.needs_you);
}

/// The settle push respects the matrix: defaults (paired phone) push
/// `agent_settled` once even with `agent_end` also on; an explicit off matrix
/// sends nothing.
#[tokio::test]
async fn settle_pushes_once_and_only_when_enabled() {
    let (server, seen) = fake_ntfy().await;
    let fixture = Fixture::new();
    fixture.pair_phone();
    // No agent_settled switch (default on); agent_end explicitly on too —
    // still exactly one "finished".
    fixture.configure(&server, r#"{"agent_end":{"enabled":true,"platforms":[]}}"#);
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();
    let paths = fixture.paths();

    let settled = ClientEvent::Session(AgentSessionEvent::AgentSettled { aborted: false });
    notify::on_session_event(&hub, &paths, &settled, &sid).await;
    wait_for_request(&seen, 1).await;
    let body = first_json(&seen).unwrap();
    let payload = decrypt(&paths, body["message"].as_str().unwrap());
    assert_eq!(payload["kind"], "agent_settled");
    assert_eq!(payload["title"], "Oino — Agent Complete");
    assert_eq!(payload["body"], "Agent is complete");

    // An explicit off matrix silences the default, paired or not.
    let (server2, seen2) = fake_ntfy().await;
    fixture.configure(
        &server2,
        r#"{"agent_settled":{"enabled":false},"agent_end":{"enabled":false}}"#,
    );
    notify::on_session_event(&hub, &paths, &settled, &sid).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(seen2.lock().unwrap().len(), 0, "disabled matrix: no push");
}

/// `notify_user` tool calls push their message to the paired phone whatever
/// platforms the tool picked (the platforms route the other surfaces); with no
/// phone paired nothing is sent.
#[tokio::test]
async fn notify_user_tool_pushes_to_the_paired_phone_by_default() {
    let (server, seen) = fake_ntfy().await;
    let fixture = Fixture::new();
    fixture.pair_phone();
    fixture.configure(&server, "{}");
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();
    let paths = fixture.paths();

    let mut args = oino_ai::JsonObject::new();
    args.insert("message".to_owned(), serde_json::json!("build finished"));
    args.insert("title".to_owned(), serde_json::json!("Nightly"));
    let tool = ClientEvent::Session(AgentSessionEvent::ToolExecutionStart {
        tool_call_id: "t1".into(),
        tool_name: "notify_user".into(),
        args,
        parent_tool_call_id: None,
    });
    notify::on_session_event(&hub, &paths, &tool, &sid).await;
    wait_for_request(&seen, 1).await;
    let body = first_json(&seen).unwrap();
    let payload = decrypt(&paths, body["message"].as_str().unwrap());
    assert_eq!(payload["kind"], "notify_user");
    assert_eq!(payload["title"], "Nightly");
    assert_eq!(payload["body"], "build finished");

    // The tool's own platform choice never excludes the phone (ruling).
    let mut native = oino_ai::JsonObject::new();
    native.insert("message".to_owned(), serde_json::json!("desktop only"));
    native.insert("platforms".to_owned(), serde_json::json!(["native"]));
    let tool = ClientEvent::Session(AgentSessionEvent::ToolExecutionStart {
        tool_call_id: "t2".into(),
        tool_name: "notify_user".into(),
        args: native,
        parent_tool_call_id: None,
    });
    notify::on_session_event(&hub, &paths, &tool, &sid).await;
    wait_for_request(&seen, 2).await;

    // No phone paired (no devices, no endpoints): nothing at all.
    drop(_env);
    let (server2, seen2) = fake_ntfy().await;
    let bare = Fixture::new();
    bare.configure(&server2, "{}");
    let _env2 = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            bare.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            bare.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let bare_hub = bare.hub();
    let bare_session = create(&bare_hub).await;
    prime(&bare_hub, &bare_session).await;
    notify::on_session_event(&bare_hub, &bare.paths(), &tool, &bare_session.session_id()).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(seen2.lock().unwrap().len(), 0, "no phone paired: no push");
}

/// The UnifiedPush fan-out (a registered endpoint also counts as the paired
/// phone the ruling needs): every registered endpoint gets the same encrypted
/// body (without the `topic` field — the URL already names it), default matrix
/// on.
#[tokio::test]
async fn unifiedpush_endpoints_are_fanned_out() {
    let (server, seen) = fake_ntfy().await;
    let (endpoint_server, endpoint_seen) = fake_ntfy().await;
    let fixture = Fixture::new();
    fixture.configure(&server, "{}");
    std::fs::write(
        &fixture.endpoints,
        format!(r#"{{"endpoints":[{{"url":"{endpoint_server}/up-topic","addedAt":0}}]}}"#),
    )
    .unwrap();
    let _env = EnvGuard::set(&[
        (
            notify::UNIPI_NOTIFY_CONFIG_ENV,
            fixture.settings.as_os_str().to_owned(),
        ),
        (
            notify::UNIPI_NOTIFY_ENDPOINTS_ENV,
            fixture.endpoints.as_os_str().to_owned(),
        ),
    ]);
    let hub = fixture.hub();
    let session = create(&hub).await;
    prime(&hub, &session).await;
    let sid = session.session_id();

    notify::on_session_event(
        &hub,
        &fixture.paths(),
        &dialog("d-1", "input", serde_json::json!({"title": "Name?"})),
        &sid,
    )
    .await;

    wait_for_request(&seen, 1).await;
    wait_for_request(&endpoint_seen, 1).await;
    let endpoint_body = first_json(&endpoint_seen).unwrap();
    assert!(
        endpoint_body.get("topic").is_none(),
        "endpoint URL names the topic"
    );
    assert_eq!(endpoint_body["title"], "UniPi");
    // Same E2E envelope shape — the phone decrypts both identically.
    let payload = decrypt(&fixture.paths(), endpoint_body["message"].as_str().unwrap());
    assert_eq!(payload["kind"], "input_needed");
}
