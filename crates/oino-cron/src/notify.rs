//! Per-job run notifications (ntfy / gotify / telegram).
//!
//! A job's `notify` field picks channels and which run statuses trigger a message
//! (`events`, default `["failed"]`, overridable by the scheduler settings'
//! `notify_default_events`). Delivery is best-effort and happens *after* the run record
//! is written: a failing channel is recorded in the run's `notify_errors`, never turns a
//! run into a failure.
//!
//! The channel connections come from the `unipi.notify` extension settings
//! (`~/.oino/settings.json` → `extensionSettings.notify`), with secret fields
//! (`gotify.appToken`, `telegram.botToken`, `telegram.chatId`, `ntfy.token`) read from
//! the secret store (`~/.oino/secrets.json`, keys `unipi.notify/<field>`), exactly as
//! the notify component resolves them.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{CronSettings, Job, Run, RunStatus};
use crate::paths::CronPaths;

/// Seconds a channel send may take.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The notify extension whose settings/secrets carry the channel connections.
const NOTIFY_EXTENSION_ID: &str = "unipi.notify";

/// The event name a finished run's status maps to, for matching against `notify.events`.
/// `None` for statuses that never trigger a notification.
#[must_use]
pub fn event_for_status(status: RunStatus) -> Option<&'static str> {
    match status {
        RunStatus::Ok => Some("done"),
        RunStatus::Failed | RunStatus::Timeout => Some("failed"),
        RunStatus::Skipped => Some("skipped"),
        RunStatus::Running | RunStatus::Queued => None,
    }
}

/// `notify.channels` if set, else the user's enabled channels.
#[must_use]
pub fn resolve_channels(job: &Job, default_channels: &[String]) -> Vec<String> {
    job.notify
        .as_ref()
        .and_then(|n| n.channels.clone())
        .unwrap_or_else(|| default_channels.to_vec())
}

/// `notify.events` if the job sets them, else the settings default, else `failed`.
#[must_use]
pub fn resolve_events_with(job: &Job, defaults: &[String]) -> Vec<String> {
    if let Some(events) = job.notify.as_ref().and_then(|n| n.events.clone()) {
        return events;
    }
    if !defaults.is_empty() {
        return defaults.to_vec();
    }
    vec!["failed".to_owned()]
}

/// Whether this run's status should notify.
#[must_use]
pub fn should_notify_with(job: &Job, status: RunStatus, defaults: &[String]) -> bool {
    let Some(event) = event_for_status(status) else {
        return false;
    };
    resolve_events_with(job, defaults)
        .iter()
        .any(|e| e == event)
}

const SUMMARY_CHARS: usize = 500;

/// "Cron: <job name> <status>" + the run summary (first ~500 chars) with cost/duration
/// appended — the same text goes to every channel.
#[must_use]
pub fn build_message(job: &Job, run: &Run) -> (String, String) {
    let title = format!("Cron: {} {}", job.name, run.status.as_str());
    let body_text = run
        .summary
        .as_deref()
        .or(run.reason.as_deref())
        .unwrap_or("(no summary)");
    let clipped: String = body_text.chars().take(SUMMARY_CHARS).collect();
    let mut extras = Vec::new();
    if let Some(cost) = run.cost_usd {
        extras.push(format!("cost ${cost:.4}"));
    }
    if let (Some(started), Some(ended)) = (run.started_at, run.ended_at) {
        extras.push(format!(
            "duration {}s",
            ended.saturating_sub(started) / 1000
        ));
    }
    let body = if extras.is_empty() {
        clipped
    } else {
        format!("{clipped}\n\n{}", extras.join(" · "))
    };
    (title, body)
}

/// A gotify connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GotifyConnection {
    /// Enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Server URL.
    #[serde(default)]
    pub server_url: Option<String>,
    /// App token (secret; usually absent from settings).
    #[serde(default)]
    pub app_token: Option<String>,
    /// Priority 0–10.
    #[serde(default)]
    pub priority: i64,
}

/// A telegram connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramConnection {
    /// Enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Bot token (secret).
    #[serde(default)]
    pub bot_token: Option<String>,
    /// Chat id (secret).
    #[serde(default)]
    pub chat_id: Option<String>,
}

/// An ntfy connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NtfyConnection {
    /// Enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Server (default `https://ntfy.sh`).
    #[serde(default)]
    pub server_url: Option<String>,
    /// Topic.
    #[serde(default)]
    pub topic: Option<String>,
    /// Access token (secret).
    #[serde(default)]
    pub token: Option<String>,
    /// Priority 1–5.
    #[serde(default)]
    pub priority: i64,
}

/// Every connection cron notifications might use, resolved once per run.
#[derive(Clone, Debug, Default)]
pub struct Channels {
    /// gotify.
    pub gotify: GotifyConnection,
    /// telegram.
    pub telegram: TelegramConnection,
    /// ntfy.
    pub ntfy: NtfyConnection,
    /// The default events for jobs that set none.
    pub default_events: Vec<String>,
}

impl Channels {
    /// Reads the channel connections from the notify extension's settings + secrets.
    #[must_use]
    pub fn load(paths: &CronPaths, settings: &CronSettings) -> Self {
        let notify = notify_settings(paths);
        let secrets = secrets(paths);
        let secret = |key: &str| {
            secrets
                .get(&format!("{NOTIFY_EXTENSION_ID}/{key}"))
                .cloned()
        };
        let mut gotify: GotifyConnection = notify
            .get("gotify")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        gotify.app_token = gotify
            .app_token
            .or_else(|| secret("gotify.appToken"))
            .filter(|t| !t.is_empty());
        let mut telegram: TelegramConnection = notify
            .get("telegram")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        telegram.bot_token = telegram
            .bot_token
            .or_else(|| secret("telegram.botToken"))
            .filter(|t| !t.is_empty());
        telegram.chat_id = telegram
            .chat_id
            .or_else(|| secret("telegram.chatId"))
            .filter(|t| !t.is_empty());
        let mut ntfy: NtfyConnection = notify
            .get("ntfy")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        ntfy.token = ntfy
            .token
            .or_else(|| secret("ntfy.token"))
            .filter(|t| !t.is_empty());
        Self {
            gotify,
            telegram,
            ntfy,
            default_events: settings.notify_default_events.clone(),
        }
    }

    /// The channels the user has enabled and configured.
    #[must_use]
    pub fn default_channels(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ntfy.enabled && self.ntfy.server_url.is_some() && self.ntfy.topic.is_some() {
            out.push("ntfy".to_owned());
        }
        if self.gotify.enabled
            && self.gotify.server_url.is_some()
            && self.gotify.app_token.is_some()
        {
            out.push("gotify".to_owned());
        }
        if self.telegram.enabled
            && self.telegram.bot_token.is_some()
            && self.telegram.chat_id.is_some()
        {
            out.push("telegram".to_owned());
        }
        out
    }
}

fn notify_settings(paths: &CronPaths) -> serde_json::Map<String, Value> {
    let settings: Option<Value> = crate::paths::read_json(&paths.agent_dir.join("settings.json"))
        .ok()
        .flatten();
    settings
        .and_then(|v| {
            v.get("extensionSettings")
                .and_then(|e| e.get("notify"))
                .and_then(Value::as_object)
                .cloned()
        })
        .unwrap_or_default()
}

fn secrets(paths: &CronPaths) -> BTreeMap<String, String> {
    crate::paths::read_json::<BTreeMap<String, String>>(&paths.agent_dir.join("secrets.json"))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Sends the run-finished notification on every selected channel, if `run.status` is one
/// of the job's events. Never fails: a channel that fails is just a string in the
/// returned list, meant for `Run.notify_errors`.
pub async fn dispatch(job: &Job, run: &Run, channels: &Channels) -> Vec<String> {
    if !should_notify_with(job, run.status, &channels.default_events) {
        return Vec::new();
    }
    let selected = resolve_channels(job, &channels.default_channels());
    if selected.is_empty() {
        return Vec::new();
    }
    let (title, body) = build_message(job, run);
    let mut errors = Vec::new();
    for channel in &selected {
        let result = match channel.as_str() {
            "ntfy" => send_ntfy(&channels.ntfy, &title, &body).await,
            "gotify" => send_gotify(&channels.gotify, &title, &body).await,
            "telegram" => send_telegram(&channels.telegram, &title, &body).await,
            other => Err(format!("unknown notify channel {other:?}")),
        };
        if let Err(error) = result {
            errors.push(format!("{channel}: {error}"));
        }
    }
    errors
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(SEND_TIMEOUT)
        .build()
        .map_err(|e| format!("http client: {e}"))
}

async fn send_ntfy(conn: &NtfyConnection, title: &str, body: &str) -> Result<(), String> {
    let Some(server) = conn.server_url.as_deref().filter(|s| !s.is_empty()) else {
        return Err("no ntfy server configured".to_owned());
    };
    let Some(topic) = conn.topic.as_deref().filter(|t| !t.is_empty()) else {
        return Err("no ntfy topic configured".to_owned());
    };
    let server = server.trim_end_matches('/');
    let mut request = client()?
        .post(format!("{server}/{topic}"))
        .json(&serde_json::json!({
            "title": title,
            "message": body,
            "tags": ["cron"],
        }));
    if let Some(token) = conn.token.as_deref().filter(|t| !t.is_empty()) {
        request = request.bearer_auth(token);
    }
    let status = request.send().await.map_err(|e| e.to_string())?.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("ntfy replied {status}"))
    }
}

async fn send_gotify(conn: &GotifyConnection, title: &str, body: &str) -> Result<(), String> {
    let Some(server) = conn.server_url.as_deref().filter(|s| !s.is_empty()) else {
        return Err("no gotify server configured".to_owned());
    };
    let Some(token) = conn.app_token.as_deref().filter(|t| !t.is_empty()) else {
        return Err("no gotify app token".to_owned());
    };
    let server = server.trim_end_matches('/');
    let status = client()?
        .post(format!("{server}/message?token={token}"))
        .json(&serde_json::json!({
            "title": title,
            "message": body,
            "priority": conn.priority,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("gotify replied {status}"))
    }
}

async fn send_telegram(conn: &TelegramConnection, title: &str, body: &str) -> Result<(), String> {
    let Some(token) = conn.bot_token.as_deref().filter(|t| !t.is_empty()) else {
        return Err("no telegram bot token".to_owned());
    };
    let Some(chat) = conn.chat_id.as_deref().filter(|c| !c.is_empty()) else {
        return Err("no telegram chat id".to_owned());
    };
    let status = client()?
        .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
        .json(&serde_json::json!({
            "chat_id": chat,
            "text": format!("{title}\n{body}"),
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("telegram replied {status}"))
    }
}

/// The path of the file `secrets` reads (tests).
#[must_use]
pub fn secrets_path(agent_dir: &Path) -> std::path::PathBuf {
    agent_dir.join("secrets.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(notify: Option<crate::model::NotifySpec>) -> Job {
        let mut job = Job::new("j", "digest", "p", "0 0 * * * *", "UTC", "/tmp");
        job.notify = notify;
        job
    }

    fn run(status: RunStatus) -> Run {
        Run {
            id: "r1".into(),
            job_id: "j".into(),
            status,
            fired_at: 0,
            started_at: Some(0),
            ended_at: Some(2500),
            exit_code: Some(0),
            summary: Some("did the thing".into()),
            reason: None,
            session_path: None,
            cost_usd: Some(0.0123),
            missed: false,
            pid: None,
            notify_errors: Vec::new(),
        }
    }

    #[test]
    fn default_events_is_failed_only() {
        let j = job(None);
        assert!(!should_notify_with(&j, RunStatus::Ok, &[]));
        assert!(should_notify_with(&j, RunStatus::Failed, &[]));
        assert!(should_notify_with(&j, RunStatus::Timeout, &[]));
        assert!(!should_notify_with(&j, RunStatus::Skipped, &[]));
        assert!(!should_notify_with(&j, RunStatus::Running, &[]));
    }

    #[test]
    fn explicit_events_widen_or_narrow_the_default() {
        let j = job(Some(crate::model::NotifySpec {
            channels: None,
            events: Some(vec!["done".to_owned(), "skipped".to_owned()]),
        }));
        assert!(should_notify_with(&j, RunStatus::Ok, &[]));
        assert!(!should_notify_with(&j, RunStatus::Failed, &[]));
        assert!(should_notify_with(&j, RunStatus::Skipped, &[]));
    }

    #[test]
    fn channels_fall_back_to_the_users_enabled_set() {
        let j = job(None);
        assert_eq!(
            resolve_channels(&j, &["ntfy".to_owned(), "telegram".to_owned()]),
            vec!["ntfy".to_owned(), "telegram".to_owned()]
        );
        let j2 = job(Some(crate::model::NotifySpec {
            channels: Some(vec!["gotify".to_owned()]),
            events: None,
        }));
        assert_eq!(
            resolve_channels(&j2, &["ntfy".to_owned()]),
            vec!["gotify".to_owned()]
        );
    }

    #[test]
    fn the_settings_default_events_apply_when_a_job_sets_none() {
        let j = job(None);
        assert!(!should_notify_with(&j, RunStatus::Ok, &[]));
        assert!(should_notify_with(&j, RunStatus::Ok, &["done".to_owned()]));
        let own = job(Some(crate::model::NotifySpec {
            channels: None,
            events: Some(vec!["failed".to_owned()]),
        }));
        assert!(!should_notify_with(
            &own,
            RunStatus::Ok,
            &["done".to_owned()]
        ));
    }

    #[test]
    fn message_has_title_summary_cost_and_duration() {
        let j = job(None);
        let r = run(RunStatus::Ok);
        let (title, body) = build_message(&j, &r);
        assert_eq!(title, "Cron: digest ok");
        assert!(body.contains("did the thing"), "body: {body}");
        assert!(body.contains("$0.0123"), "body: {body}");
        assert!(body.contains("duration 2s"), "body: {body}");
    }

    #[test]
    fn message_falls_back_to_reason_with_no_summary() {
        let j = job(None);
        let mut r = run(RunStatus::Failed);
        r.summary = None;
        r.reason = Some("oino exited with status 1".into());
        let (_, body) = build_message(&j, &r);
        assert!(
            body.starts_with("oino exited with status 1"),
            "body: {body}"
        );
    }

    #[test]
    fn summary_is_clipped_to_500_chars() {
        let j = job(None);
        let mut r = run(RunStatus::Ok);
        r.summary = Some("x".repeat(900));
        let (_, body) = build_message(&j, &r);
        let summary_part = body.split("\n\n").next().unwrap();
        assert_eq!(summary_part.chars().count(), 500);
    }

    #[test]
    fn channels_load_from_notify_settings_and_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CronPaths::new(tmp.path());
        std::fs::write(
            paths.agent_dir.join("settings.json"),
            serde_json::json!({
                "extensionSettings": {
                    "notify": {
                        "gotify": {"enabled": true, "serverUrl": "https://gotify.example", "priority": 5},
                        "ntfy": {"enabled": true, "serverUrl": "https://ntfy.example", "topic": "mytopic", "priority": 3},
                        "telegram": {"enabled": false}
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            secrets_path(&paths.agent_dir),
            serde_json::json!({"unipi.notify/gotify.appToken": "T0K3N"}).to_string(),
        )
        .unwrap();
        let channels = Channels::load(&paths, &CronSettings::default());
        assert_eq!(channels.default_channels(), vec!["ntfy", "gotify"]);
        assert_eq!(channels.gotify.app_token.as_deref(), Some("T0K3N"));
        assert_eq!(channels.ntfy.topic.as_deref(), Some("mytopic"));
    }

    #[tokio::test]
    async fn dispatch_records_a_failing_channel_without_panicking() {
        let j = job(Some(crate::model::NotifySpec {
            channels: Some(vec!["gotify".to_owned()]),
            events: Some(vec!["done".to_owned()]),
        }));
        let r = run(RunStatus::Ok);
        let channels = Channels {
            gotify: GotifyConnection {
                enabled: true,
                server_url: Some("http://127.0.0.1:1".to_owned()),
                app_token: Some("t".to_owned()),
                priority: 5,
            },
            ..Channels::default()
        };
        let errors = dispatch(&j, &r, &channels).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("gotify:"), "errors: {errors:?}");
    }
}
