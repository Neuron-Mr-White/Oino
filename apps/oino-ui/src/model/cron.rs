//! Automations (cron) view model: decode of the scheduler's control-op replies
//! (`oino_cron::control`, the `list`/`status`/`runs`/`show` payloads).

use serde::Deserialize;
use serde_json::Value;

/// One job row (`list` + `status` merged).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CronJob {
    /// Job id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Schedule (machine form: 6-field cron or `at:`).
    pub schedule: String,
    /// IANA zone.
    pub tz: String,
    /// Enabled.
    pub enabled: bool,
    /// A run is live right now.
    #[serde(default)]
    pub running: bool,
    /// Next fire (RFC3339), enabled jobs only.
    #[serde(default)]
    pub next_fire: Option<String>,
    /// Newest recorded run's status (`status` reply).
    #[serde(default)]
    pub last_status: Option<String>,
    /// Newest recorded run's fire time (unix ms).
    #[serde(default)]
    pub last_fire: Option<u64>,
    /// Model override.
    #[serde(default)]
    pub model: Option<String>,
    /// Working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The job's prompt.
    #[serde(default)]
    pub prompt: Option<String>,
}

impl CronJob {
    /// The schedule rendered for the list (`at:` stripped, seconds field dropped).
    #[must_use]
    pub fn schedule_pretty(&self) -> String {
        if let Some(at) = self.schedule.strip_prefix("at:") {
            return format!("at {at}");
        }
        let fields: Vec<&str> = self.schedule.split_whitespace().collect();
        if fields.len() == 6 {
            fields[1..].join(" ")
        } else {
            self.schedule.clone()
        }
    }

    /// Short next-fire text (local time "MM-dd HH:MM").
    #[must_use]
    pub fn next_fire_short(&self) -> String {
        short_time(self.next_fire.as_deref())
    }

    /// "<status> <MM-dd HH:MM>" of the newest run, `—` when there is none.
    #[must_use]
    pub fn last_run_text(&self) -> String {
        let Some(status) = &self.last_status else {
            return "—".to_owned();
        };
        format!(
            "{status} {}",
            short_time(self.last_fire.and_then(millis_to_rfc3339).as_deref())
        )
    }

    /// Whether the newest run failed.
    #[must_use]
    pub fn last_run_failed(&self) -> bool {
        matches!(self.last_status.as_deref(), Some("failed" | "timeout"))
    }
}

/// One run row.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CronRun {
    /// Run id.
    pub id: String,
    /// Job id.
    pub job_id: String,
    /// Status word.
    pub status: String,
    /// Fired at (unix ms).
    pub fired_at: u64,
    /// Started at.
    #[serde(default)]
    pub started_at: Option<u64>,
    /// Ended at.
    #[serde(default)]
    pub ended_at: Option<u64>,
    /// The run's summary text.
    #[serde(default)]
    pub summary: Option<String>,
    /// Skip/fail reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// The run's session file (openable).
    #[serde(default)]
    pub session_path: Option<String>,
    /// Cost.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Catch-up run.
    #[serde(default)]
    pub missed: bool,
}

impl CronRun {
    /// One-line summary or reason.
    #[must_use]
    pub fn summary_text(&self) -> &str {
        self.summary
            .as_deref()
            .or(self.reason.as_deref())
            .unwrap_or("")
    }

    /// Whether the status means failure.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "timeout")
    }
}

/// The whole page snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CronSnapshot {
    /// Everything paused.
    pub paused: bool,
    /// Run concurrency.
    pub concurrency: u64,
    /// Jobs.
    pub jobs: Vec<CronJob>,
}

/// Decodes the snapshot from `status` + `list` replies.
#[must_use]
pub fn snapshot(status: &Value, list: &Value) -> CronSnapshot {
    let mut jobs: Vec<CronJob> = list
        .get("jobs")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(|j| decode_job(j, status)).collect())
        .unwrap_or_default();
    jobs.sort_by(|a, b| a.id.cmp(&b.id));
    CronSnapshot {
        paused: status
            .get("paused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        concurrency: status
            .get("concurrency")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        jobs,
    }
}

fn decode_job(list_row: &Value, status: &Value) -> Option<CronJob> {
    let mut job: CronJob = serde_json::from_value(list_row.clone()).ok()?;
    let id = job.id.clone();
    if let Some(row) = status
        .get("jobs")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.get("id").and_then(Value::as_str) == Some(id.as_str()))
        })
    {
        job.running = row.get("running").and_then(Value::as_bool).unwrap_or(false);
        job.next_fire = row
            .get("next_fire")
            .and_then(Value::as_str)
            .map(str::to_owned);
        job.last_status = row
            .get("last_status")
            .and_then(Value::as_str)
            .map(str::to_owned);
        job.last_fire = row.get("last_fire").and_then(Value::as_u64);
    }
    Some(job)
}

/// Decodes runs from a `runs`/`show` reply.
#[must_use]
pub fn runs(reply: &Value) -> Vec<CronRun> {
    reply
        .get("runs")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| serde_json::from_value(r.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `2026-01-01T09:00:00+01:00` → `01-01 08:00` (UTC-agnostic short form: keep the offset
/// the scheduler reported; local rendering would need the zone, which we don't keep here).
#[must_use]
pub fn short_time(rfc3339: Option<&str>) -> String {
    let Some(text) = rfc3339 else {
        return "—".to_owned();
    };
    // "YYYY-MM-DDTHH:MM…" → "MM-DD HH:MM".
    let bytes = text.as_bytes();
    if bytes.len() >= 16 && bytes[10] == b'T' {
        format!("{} {}", &text[5..10], &text[11..16])
    } else {
        text.to_owned()
    }
}

/// `fired_at` unix ms → RFC3339 (`MM-dd HH:MM` is all the lists show; runs sort
/// newest-first regardless).
#[must_use]
#[allow(
    clippy::many_single_char_names,
    reason = "civil-from-days date arithmetic reads best with the textbook names"
)]
pub fn millis_to_rfc3339(ms: u64) -> Option<String> {
    let secs = i64::try_from(ms / 1000).ok()?;
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (h, m) = (rest / 3600, (rest % 3600) / 60);
    // Epoch-day → Y-M-D (civil-from-days algorithm).
    let z = 719_468 + days;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    Some(format!("{y:04}-{mth:02}-{d:02}T{h:02}:{m:02}:00Z"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_snapshot() {
        let status = json!({
            "paused": true,
            "concurrency": 2,
            "jobs": [
                {"id": "digest", "enabled": true, "next_fire": "2026-01-02T09:00:00+01:00", "running": true},
            ],
        });
        let list = json!({"jobs": [
            {"id": "digest", "name": "Digest", "schedule": "0 0 9 * * *", "tz": "Europe/Berlin", "enabled": true, "cwd": "/tmp", "prompt": "p"},
            {"id": "check", "name": "Check", "schedule": "at:2026-01-01T12:00:00Z", "tz": "UTC", "enabled": false},
        ]});
        let snap = snapshot(&status, &list);
        assert!(snap.paused);
        assert_eq!(snap.jobs.len(), 2);
        let digest = snap.jobs.iter().find(|j| j.id == "digest").unwrap();
        assert!(digest.running);
        assert_eq!(digest.schedule_pretty(), "0 9 * * *");
        assert_eq!(digest.next_fire_short(), "01-02 09:00");
        let check = snap.jobs.iter().find(|j| j.id == "check").unwrap();
        assert_eq!(check.schedule_pretty(), "at 2026-01-01T12:00:00Z");
        assert_eq!(check.next_fire_short(), "—");
    }

    #[test]
    fn decodes_runs() {
        let reply = json!({"runs": [
            {"id": "r1", "job_id": "j", "status": "ok", "fired_at": 0, "summary": "done",
             "session_path": "/s.jsonl", "cost_usd": 0.1},
            {"id": "r2", "job_id": "j", "status": "failed", "fired_at": 1, "reason": "boom"},
        ]});
        let runs = runs(&reply);
        assert_eq!(runs.len(), 2);
        assert!(!runs[0].is_failure());
        assert!(runs[1].is_failure());
        assert_eq!(runs[1].summary_text(), "boom");
    }
}
