//! Per-job cron notifications (UNI-316, Cron C3).
//!
//! A job's `notify` field picks channels (`ntfy`/`gotify`/`telegram`) and
//! which run statuses trigger a message (`events`, default `["failed"]`).
//! Delivery is best-effort and happens *after* the run record is already
//! written: a failing channel is recorded in the run's `notify_errors`,
//! never turns a run itself into a failure.
//!
//! - ntfy reuses [`crate::notify::send_payload`]/`encrypt_envelope` with
//!   the host's own E2E notify key, so the phone decrypts it exactly like
//!   any other UniPi push. `cron.ntfy_topic` (the `cron` settings block)
//!   lets it ride a separate topic from the user's normal one.
//! - gotify/telegram go through [`crate::notify::send_gotify`] /
//!   `send_telegram`, reading the same `~/.unipi/config/notify/config.json`
//!   the TS `notify` package's Settings UI writes.

use crate::data_dir::HostPaths;
use crate::notify::{self, GotifyConnection, NtfyConnection, TelegramConnection};

use super::model::{CronSettings, Job, Run, RunStatus};

/// The event name a finished run's status maps to, for matching against
/// `notify.events`. `None` for statuses that never trigger a notification
/// (still running/queued — there's nothing to report yet).
pub fn event_for_status(status: RunStatus) -> Option<&'static str> {
    match status {
        RunStatus::Ok => Some("done"),
        RunStatus::Failed | RunStatus::Timeout => Some("failed"),
        RunStatus::Skipped => Some("skipped"),
        RunStatus::Running | RunStatus::Queued => None,
    }
}

/// `notify.channels` if set, else the user's enabled channels.
pub fn resolve_channels(job: &Job, default_channels: &[String]) -> Vec<String> {
    job.notify.as_ref().and_then(|n| n.channels.clone()).unwrap_or_else(|| default_channels.to_vec())
}

/// `notify.events` if set, else `["failed"]`.
pub fn resolve_events(job: &Job) -> Vec<String> {
    resolve_events_with(job, &[])
}

/// `notify.events` if the job sets them, else the PC's default from the
/// `cron` settings (`notify_default_events`, UNI-318), else `failed`.
pub fn resolve_events_with(job: &Job, defaults: &[String]) -> Vec<String> {
    if let Some(events) = job.notify.as_ref().and_then(|n| n.events.clone()) {
        return events;
    }
    if !defaults.is_empty() {
        return defaults.to_vec();
    }
    vec!["failed".to_string()]
}

/// Whether this run's status should notify, given the job's `notify.events`
/// (or the default).
pub fn should_notify(job: &Job, status: RunStatus) -> bool {
    should_notify_with(job, status, &[])
}

pub fn should_notify_with(job: &Job, status: RunStatus, defaults: &[String]) -> bool {
    let Some(event) = event_for_status(status) else { return false };
    resolve_events_with(job, defaults).iter().any(|e| e == event)
}

const SUMMARY_CHARS: usize = 500;

/// "Cron: <job name> <status>" + the run summary (first ~500 chars) with
/// cost/duration appended — the same text goes to every channel; ntfy's
/// payload additionally carries the structured `kind`/`job_id`/`run_id`/
/// `status` fields for the app's deep link.
pub fn build_message(job: &Job, run: &Run) -> (String, String) {
    let title = format!("Cron: {} {}", job.name, run.status.as_str());
    let body_text = run.summary.as_deref().or(run.reason.as_deref()).unwrap_or("(no summary)");
    let clipped: String = body_text.chars().take(SUMMARY_CHARS).collect();
    let mut extras = Vec::new();
    if let Some(cost) = run.cost_usd {
        extras.push(format!("cost ${cost:.4}"));
    }
    if let (Some(started), Some(ended)) = (run.started_at, run.ended_at) {
        extras.push(format!("duration {}s", ended.saturating_sub(started) / 1000));
    }
    let body = if extras.is_empty() { clipped } else { format!("{clipped}\n\n{}", extras.join(" · ")) };
    (title, body)
}

/// Every connection cron notifications might use, resolved once per run.
pub struct Channels {
    pub ntfy: NtfyConnection,
    pub endpoints: Vec<String>,
    pub key: Option<[u8; 32]>,
    pub ntfy_topic: Option<String>,
    pub gotify: GotifyConnection,
    pub telegram: TelegramConnection,
    /// The PC's default events for jobs that set none (UNI-318).
    pub default_events: Vec<String>,
}

impl Channels {
    /// Reads every channel's connection details from disk (UNI-316): the
    /// shared `~/.unipi/config/notify/config.json` for ntfy/gotify/telegram,
    /// the app-endpoints fan-out file, and the host's own E2E notify key
    /// (generated on first use).
    pub fn load(paths: &HostPaths, settings: &CronSettings) -> Channels {
        let config_path = notify::notify_config_json_path();
        Channels {
            ntfy: notify::read_ntfy_config(&config_path),
            endpoints: notify::read_app_endpoints(&notify::app_endpoints_path()),
            key: notify::notify_key_or_create(paths).ok(),
            ntfy_topic: settings.ntfy_topic.clone(),
            gotify: notify::read_gotify_config(&config_path),
            telegram: notify::read_telegram_config(&config_path),
            default_events: settings.notify_default_events.clone(),
        }
    }

    pub fn default_channels(&self) -> Vec<String> {
        notify::default_channels(&self.ntfy, &self.gotify, &self.telegram)
    }
}

/// Sends the run-finished notification on every channel `job.notify`
/// (or the default) selects, if `run.status` is one of `notify.events`.
/// Never returns an error: a channel that fails is just a string in the
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
        let result: anyhow::Result<()> = match channel.as_str() {
            "ntfy" => send_ntfy(job, run, channels, &title, &body).await,
            "gotify" => notify::send_gotify(&channels.gotify, &title, &body, 5).await,
            "telegram" => notify::send_telegram(&channels.telegram, &title, &body).await,
            other => Err(anyhow::anyhow!("unknown notify channel {other:?}")),
        };
        if let Err(error) = result {
            tracing::warn!(op = "cron_notify", job = job.id, run = run.id, channel = channel.as_str(), %error, "notification failed");
            errors.push(format!("{channel}: {error}"));
        }
    }
    errors
}

async fn send_ntfy(job: &Job, run: &Run, channels: &Channels, title: &str, body: &str) -> anyhow::Result<()> {
    let Some(key) = channels.key else {
        anyhow::bail!("no notify key");
    };
    let topic = channels.ntfy_topic.as_deref().or(channels.ntfy.topic.as_deref());
    let connection = NtfyConnection { topic: topic.map(str::to_string), ..channels.ntfy.clone() };
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "kind": "cron",
        "job_id": job.id,
        "run_id": run.id,
        "status": run.status.as_str(),
    });
    let sent = notify::send_payload(&connection, &channels.endpoints, &key, &payload).await?;
    if sent == 0 {
        anyhow::bail!("nothing to send to (no topic/token and no endpoints)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(notify: Option<crate::cron::model::NotifySpec>) -> Job {
        Job {
            id: "j".into(),
            name: "digest".into(),
            prompt: "p".into(),
            schedule: "0 0 * * * *".into(),
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
            notify_errors: Vec::new(),
            wake: None,
        }
    }

    #[test]
    fn default_events_is_failed_only() {
        let j = job(None);
        assert!(!should_notify(&j, RunStatus::Ok));
        assert!(should_notify(&j, RunStatus::Failed));
        assert!(should_notify(&j, RunStatus::Timeout));
        assert!(!should_notify(&j, RunStatus::Skipped));
        assert!(!should_notify(&j, RunStatus::Running));
    }

    #[test]
    fn explicit_events_widen_or_narrow_the_default() {
        let j = job(Some(crate::cron::model::NotifySpec {
            channels: None,
            events: Some(vec!["done".to_string(), "skipped".to_string()]),
        }));
        assert!(should_notify(&j, RunStatus::Ok));
        assert!(!should_notify(&j, RunStatus::Failed));
        assert!(should_notify(&j, RunStatus::Skipped));
    }

    #[test]
    fn channels_fall_back_to_the_users_enabled_set() {
        let j = job(None);
        assert_eq!(resolve_channels(&j, &["ntfy".to_string(), "telegram".to_string()]), vec!["ntfy".to_string(), "telegram".to_string()]);
        let j2 = job(Some(crate::cron::model::NotifySpec { channels: Some(vec!["gotify".to_string()]), events: None }));
        assert_eq!(resolve_channels(&j2, &["ntfy".to_string()]), vec!["gotify".to_string()]);
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
        r.reason = Some("pi exited with status 1".into());
        let (_, body) = build_message(&j, &r);
        assert!(body.starts_with("pi exited with status 1"), "body: {body}");
    }

    #[test]
    fn summary_is_clipped_to_500_chars() {
        let j = job(None);
        let mut r = run(RunStatus::Ok);
        r.summary = Some("x".repeat(900));
        let (_, body) = build_message(&j, &r);
        let summary_part = body.split("\n\n").next().unwrap();
        assert_eq!(summary_part.len(), 500);
    }

    #[tokio::test]
    async fn dispatch_skips_entirely_when_the_status_does_not_match_events() {
        let j = job(None); // default events: ["failed"]
        let r = run(RunStatus::Ok);
        let channels = Channels {
            ntfy: NtfyConnection::default(),
            endpoints: Vec::new(),
            key: Some([0u8; 32]),
            ntfy_topic: None,
            gotify: GotifyConnection::default(),
            telegram: TelegramConnection::default(),
            default_events: Vec::new(),
        };
        assert!(dispatch(&j, &r, &channels).await.is_empty());
    }

    #[test]
    fn the_pc_default_events_apply_when_a_job_sets_none() {
        let j = job(None);
        assert!(!should_notify_with(&j, RunStatus::Ok, &[]), "built-in default: failed only");
        assert!(should_notify_with(&j, RunStatus::Ok, &["done".to_string()]), "the PC's notify_default_events wins");
        let own = job(Some(crate::cron::model::NotifySpec { channels: None, events: Some(vec!["failed".to_string()]) }));
        assert!(!should_notify_with(&own, RunStatus::Ok, &["done".to_string()]), "a job's own events win over the default");
    }

    #[tokio::test]
    async fn dispatch_records_a_failing_channel_without_panicking() {
        let j = job(Some(crate::cron::model::NotifySpec { channels: Some(vec!["gotify".to_string()]), events: Some(vec!["done".to_string()]) }));
        let r = run(RunStatus::Ok);
        let channels = Channels {
            ntfy: NtfyConnection::default(),
            endpoints: Vec::new(),
            key: Some([0u8; 32]),
            ntfy_topic: None,
            gotify: GotifyConnection::default(), // unconfigured -> fails
            telegram: TelegramConnection::default(),
            default_events: Vec::new(),
        };
        let errors = dispatch(&j, &r, &channels).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("gotify:"), "errors: {errors:?}");
    }
}
