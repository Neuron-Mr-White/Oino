//! UNI-161 \u00a71/\u00a72: `notify_config` returns the host's own ntfy connection
//! plus the per-pairing E2E key; `notify_register` writes the phone's
//! UnifiedPush endpoint into app-endpoints.json in the shape notify's
//! fan-out already reads.

use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, write_control, Handler},
};
use unipi_proto::{Control, Request};

/// Serializes access to the two notify env-var overrides so tests that
/// set them never race each other (same idea as data_dir's EnvGuard).
static NOTIFY_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(Vec::new()),
        board: Default::default(),
    }
}

async fn request(host: Handler, remote: iroh::EndpointId, req: Request) -> Control {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let task = tokio::spawn(async move {
        host.handle(read, write, remote).await.unwrap();
    });
    let (mut read, mut write) = tokio::io::split(client);
    let hello = Control::Hello {
        proto: "1.0.0".to_owned().try_into().unwrap(),
        req,
    };
    write_control(&mut write, &hello).await.unwrap();
    let (_, bytes) = read_frame(&mut read).await.unwrap();
    task.await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn notify_config_returns_key_and_ntfy_connection() {
    let _lock = NOTIFY_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("config.json");
    std::fs::write(
        &config_path,
        r#"{"ntfy":{"enabled":true,"serverUrl":"https://ntfy.nrn.one","topic":"abc123","token":"tk_secret","appDetail":"full"}}"#,
    )
    .unwrap();
    std::env::set_var(unipi_host::notify::UNIPI_NOTIFY_CONFIG_ENV, &config_path);

    let paths = HostPaths::from_dir(data_dir.path());
    let host = handler(paths.clone());
    let devices = DeviceStore::new(paths.devices());
    let remote = iroh::SecretKey::generate().public();
    devices.add(&remote, "Phone".into()).unwrap();

    let reply = request(host, remote, Request::NotifyConfig).await;
    std::env::remove_var(unipi_host::notify::UNIPI_NOTIFY_CONFIG_ENV);

    let Control::Ok { data: Some(data) } = reply else {
        panic!("expected ok with data, got {reply:?}");
    };
    assert_eq!(data["server"], "https://ntfy.nrn.one");
    assert_eq!(data["topic"], "abc123");
    assert_eq!(data["token"], "tk_secret");
    assert_eq!(data["app_detail"], "full");
    let key_b64 = data["key"].as_str().unwrap();
    use base64::Engine;
    let key_bytes = base64::engine::general_purpose::STANDARD.decode(key_b64).unwrap();
    assert_eq!(key_bytes.len(), 32);

    // The key is persisted in the host's data dir, same bytes next time.
    let persisted = unipi_host::notify::notify_key_or_create(&paths).unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.encode(persisted), key_b64);
}

#[tokio::test]
async fn notify_config_without_config_json_omits_server_but_still_returns_a_key() {
    let _lock = NOTIFY_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = tempfile::tempdir().unwrap();
    let missing = tempfile::tempdir().unwrap().path().join("nope").join("config.json");
    std::env::set_var(unipi_host::notify::UNIPI_NOTIFY_CONFIG_ENV, &missing);

    let paths = HostPaths::from_dir(data_dir.path());
    let host = handler(paths.clone());
    let devices = DeviceStore::new(paths.devices());
    let remote = iroh::SecretKey::generate().public();
    devices.add(&remote, "Phone".into()).unwrap();

    let reply = request(host, remote, Request::NotifyConfig).await;
    std::env::remove_var(unipi_host::notify::UNIPI_NOTIFY_CONFIG_ENV);

    let Control::Ok { data: Some(data) } = reply else {
        panic!("expected ok with data, got {reply:?}");
    };
    assert!(data["server"].is_null());
    assert!(data["topic"].is_null());
    assert!(data["key"].as_str().is_some());
}

#[tokio::test]
async fn notify_register_writes_app_endpoints_json() {
    let _lock = NOTIFY_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let data_dir = tempfile::tempdir().unwrap();
    let endpoints_dir = tempfile::tempdir().unwrap();
    let endpoints_path = endpoints_dir.path().join("app-endpoints.json");
    std::env::set_var(unipi_host::notify::UNIPI_NOTIFY_ENDPOINTS_ENV, &endpoints_path);

    let paths = HostPaths::from_dir(data_dir.path());
    let host = handler(paths.clone());
    let devices = DeviceStore::new(paths.devices());
    let remote = iroh::SecretKey::generate().public();
    devices.add(&remote, "Phone".into()).unwrap();

    let endpoint: unipi_proto::NotifyEndpoint =
        "https://ntfy.sh/unipi-phone-endpoint-abc".to_string().try_into().unwrap();
    let reply = request(host, remote, Request::NotifyRegister { endpoint }).await;
    std::env::remove_var(unipi_host::notify::UNIPI_NOTIFY_ENDPOINTS_ENV);

    assert!(matches!(reply, Control::Ok { .. }));
    let raw = std::fs::read_to_string(&endpoints_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let endpoints = parsed["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0]["url"], "https://ntfy.sh/unipi-phone-endpoint-abc");
}

#[tokio::test]
async fn notify_requests_require_pairing() {
    let data_dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(data_dir.path());
    let host = handler(paths);
    let remote = iroh::SecretKey::generate().public();
    let reply = request(host, remote, Request::NotifyConfig).await;
    match reply {
        Control::Err { error } => assert_eq!(error.code, unipi_proto::AppErrorCode::E_UNAUTHORIZED),
        other => panic!("expected E_UNAUTHORIZED, got {other:?}"),
    }
}
