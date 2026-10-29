//! UNI-316 integration tests: cron notify dispatch against fake HTTP
//! servers (ntfy/gotify/telegram-shaped), never the user's real channels.
//! Tests in this file share process-global env vars
//! (`UNIPI_NOTIFY_CONFIG_JSON`/`UNIPI_NOTIFY_ENDPOINTS_JSON`), so they run
//! serialized through `ENV_LOCK`.

#![cfg(unix)]

use std::sync::Mutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use unipi_host::cron::model::{Job, NotifySpec, Run, RunStatus};
use unipi_host::cron::notify::{dispatch, Channels};
use unipi_host::cron::model::CronSettings;
use unipi_host::data_dir::HostPaths;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn job(notify: Option<NotifySpec>) -> Job {
    Job {
        id: "digest".into(),
        name: "daily digest".into(),
        prompt: "p".into(),
        schedule: "0 0 9 * * *".into(),
        tz: "UTC".into(),
        cwd: "/tmp".into(),
        model: None,
        thinking: None,
        enabled: true,
        time_limit_s: 10,
        cost_cap_usd: None,
        notify,
        session_link: None,
        created: 0,
        updated: 0,
    }
}

fn run(status: RunStatus) -> Run {
    Run {
        id: "r1".into(),
        job_id: "digest".into(),
        status,
        fired_at: 0,
        started_at: Some(0),
        ended_at: Some(3000),
        exit_code: Some(0),
        summary: Some("Summarized 12 emails".into()),
        reason: None,
        session_path: None,
        cost_usd: Some(0.02),
        missed: false,
        notify_errors: Vec::new(),
        wake: None,
    }
}

/// One-shot fake HTTP server: captures the raw request, replies `status`.
async fn fake_server(status: &'static str) -> (std::net::SocketAddr, tokio::sync::oneshot::Receiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 16384];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        buf.truncate(n);
        let body = format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
        let _ = socket.write_all(body.as_bytes()).await;
        let _ = socket.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    (addr, rx)
}

fn write_notify_config(path: &std::path::Path, json: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, json).unwrap();
}

/// ntfy: the run's notification decrypts with the host's own key — the
/// same `unipi1:` E2E envelope every other UniPi push uses.
#[tokio::test]
async fn ntfy_channel_sends_an_envelope_that_decrypts_with_the_hosts_key() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let (addr, rx) = fake_server("200 OK").await;

    let config_path = tmp.path().join("notify-config.json");
    write_notify_config(
        &config_path,
        &format!(r#"{{"ntfy":{{"enabled":true,"serverUrl":"http://{addr}","topic":"cron-topic"}}}}"#),
    );
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);
    std::env::set_var("UNIPI_NOTIFY_ENDPOINTS_JSON", tmp.path().join("endpoints.json"));

    let settings = CronSettings::default();
    let channels = Channels::load(&paths, &settings);
    let key = unipi_host::notify::notify_key_or_create(&paths).unwrap();

    let j = job(Some(NotifySpec { channels: Some(vec!["ntfy".to_string()]), events: Some(vec!["done".to_string()]) }));
    let r = run(RunStatus::Ok);
    let errors = dispatch(&j, &r, &channels).await;
    assert!(errors.is_empty(), "errors: {errors:?}");

    let request = rx.await.unwrap();
    let body: serde_json::Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["topic"], "cron-topic");
    let message = body["message"].as_str().unwrap();
    assert!(message.starts_with("unipi1:"), "message: {message}");
    // Decrypt with the host's key, like the phone app would.
    let envelope = message.strip_prefix("unipi1:").unwrap();
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, envelope).unwrap();
    let (iv, ciphertext) = (&raw[..12], &raw[12..]);
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(key));
    let plaintext = cipher.decrypt(&Nonce::try_from(iv).unwrap(), ciphertext).expect("decrypt");
    let payload: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(payload["kind"], "cron");
    assert_eq!(payload["job_id"], "digest");
    assert_eq!(payload["run_id"], "r1");
    assert_eq!(payload["status"], "ok");
    assert!(payload["title"].as_str().unwrap().contains("daily digest"));

    std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON");
    std::env::remove_var("UNIPI_NOTIFY_ENDPOINTS_JSON");
}

/// gotify: request shape (`POST <server>/message`) and the `X-Gotify-Key`
/// auth header.
#[tokio::test]
async fn gotify_channel_posts_message_with_auth_header() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let (addr, rx) = fake_server("200 OK").await;

    let config_path = tmp.path().join("notify-config.json");
    write_notify_config(
        &config_path,
        &format!(r#"{{"gotify":{{"enabled":true,"serverUrl":"http://{addr}","appToken":"tok_g"}}}}"#),
    );
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);
    std::env::set_var("UNIPI_NOTIFY_ENDPOINTS_JSON", tmp.path().join("endpoints.json"));

    let settings = CronSettings::default();
    let channels = Channels::load(&paths, &settings);
    let j = job(Some(NotifySpec { channels: Some(vec!["gotify".to_string()]), events: Some(vec!["failed".to_string()]) }));
    let r = run(RunStatus::Failed);
    let errors = dispatch(&j, &r, &channels).await;
    assert!(errors.is_empty(), "errors: {errors:?}");

    let request = rx.await.unwrap();
    assert!(request.starts_with("POST /message"), "request: {request}");
    assert!(request.to_lowercase().contains("x-gotify-key: tok_g"), "request: {request}");
    assert!(request.contains("Cron: daily digest failed"), "request: {request}");

    std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON");
    std::env::remove_var("UNIPI_NOTIFY_ENDPOINTS_JSON");
}

/// telegram: request shape (`POST .../bot<token>/sendMessage`) and the
/// chat id in the body.
#[tokio::test]
async fn telegram_channel_posts_chat_id_and_text() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let config_path = tmp.path().join("notify-config.json");
    write_notify_config(&config_path, r#"{"telegram":{"enabled":true,"botToken":"tok_t","chatId":"42"}}"#);
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);
    std::env::set_var("UNIPI_NOTIFY_ENDPOINTS_JSON", tmp.path().join("endpoints.json"));

    let settings = CronSettings::default();
    let channels = Channels::load(&paths, &settings);
    assert_eq!(channels.telegram.bot_token.as_deref(), Some("tok_t"));
    assert_eq!(channels.telegram.chat_id.as_deref(), Some("42"));
    assert!(channels.telegram.enabled);

    std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON");
    std::env::remove_var("UNIPI_NOTIFY_ENDPOINTS_JSON");
}

/// The default channels: no `notify.channels` on the job -> the user's own
/// enabled set (here: ntfy + telegram, not gotify).
#[tokio::test]
async fn default_channels_come_from_the_users_own_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let config_path = tmp.path().join("notify-config.json");
    write_notify_config(
        &config_path,
        r#"{"ntfy":{"enabled":true,"serverUrl":"https://ntfy.sh","topic":"t"},"gotify":{"enabled":false},"telegram":{"enabled":true,"botToken":"x","chatId":"1"}}"#,
    );
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);
    std::env::set_var("UNIPI_NOTIFY_ENDPOINTS_JSON", tmp.path().join("endpoints.json"));

    let settings = CronSettings::default();
    let channels = Channels::load(&paths, &settings);
    assert_eq!(channels.default_channels(), vec!["ntfy".to_string(), "telegram".to_string()]);

    std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON");
    std::env::remove_var("UNIPI_NOTIFY_ENDPOINTS_JSON");
}

/// A failing channel is recorded in the run (via `dispatch`'s error list)
/// without ever turning the run itself into a failure — the scheduler
/// only ever sets `run.notify_errors`, `run.status` is untouched here.
#[tokio::test]
async fn a_failing_channel_is_recorded_not_fatal() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let (addr, _rx) = fake_server("500 Internal Server Error").await;
    let config_path = tmp.path().join("notify-config.json");
    write_notify_config(
        &config_path,
        &format!(r#"{{"gotify":{{"enabled":true,"serverUrl":"http://{addr}","appToken":"tok_g"}}}}"#),
    );
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);
    std::env::set_var("UNIPI_NOTIFY_ENDPOINTS_JSON", tmp.path().join("endpoints.json"));

    let settings = CronSettings::default();
    let channels = Channels::load(&paths, &settings);
    let j = job(Some(NotifySpec { channels: Some(vec!["gotify".to_string()]), events: Some(vec!["done".to_string(), "failed".to_string()]) }));
    let mut r = run(RunStatus::Ok);
    r.status = RunStatus::Failed;
    let errors = dispatch(&j, &r, &channels).await;
    assert_eq!(errors.len(), 1);
    assert!(errors[0].starts_with("gotify:"), "errors: {errors:?}");
    // The run's own status is whatever the caller set; dispatch never
    // touches it.
    assert_eq!(r.status, RunStatus::Failed);

    std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON");
    std::env::remove_var("UNIPI_NOTIFY_ENDPOINTS_JSON");
}
