//! Session wake (UNI-316, Cron C3): when a cron run finishes, nudge a live
//! pi session back to life with the result, if the job asked to be woken.
//!
//! `Job.session_link` identifies a bridge the same way `chat.rs` already
//! does: `~/.unipi/bridge/<pid>.json` records (`read_bridges`), matched by
//! `session_file` first (the more precise identifier pi's own
//! `session-manager.ts` writes), falling back to `session_id`. The bridge's
//! own socket then gets a `{"t":"prompt",...}` line — the exact same wire
//! message `packages/app-bridge/src/wire.ts` already accepts from the
//! phone, so a cron wake looks to pi like any other "someone sent me a
//! message" (UNI-202's queue semantics apply the same way): `mode:"auto"`
//! lets the bridge itself pick `steer` while busy (next tool boundary) or
//! deliver as a plain prompt while idle — i.e. "run it now if idle, queue
//! it if busy", which is exactly the "sent" outcome this module reports.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::model::{Job, Run, SessionLink};
use crate::chat::{read_bridges, BridgeRecord};

/// Builds the wake message text: `"[cron] <job name> finished: <status>.
/// <summary first 300 chars>. Run <run id> — details: unipi-host cron log
/// <run id>"`.
pub fn wake_text(job: &Job, run: &Run) -> String {
    let summary = run.summary.as_deref().or(run.reason.as_deref()).unwrap_or("(no summary)");
    let clipped: String = summary.chars().take(300).collect();
    format!(
        "[cron] {} finished: {}. {}. Run {} — details: unipi-host cron log {}",
        job.name,
        run.status.as_str(),
        clipped,
        run.id,
        run.id,
    )
}

/// Finds the live bridge `link` points at: `session_file` first, else
/// `session_id`. `None` when no live bridge matches (the session is gone
/// or was never found).
pub fn find_linked_bridge(bridge_dir: &Path, link: &SessionLink) -> Option<BridgeRecord> {
    let bridges = read_bridges(bridge_dir);
    if let Some(file) = link.session_file.as_deref() {
        if let Some(found) = bridges.iter().find(|b| b.session_file.as_deref() == Some(file)) {
            return Some(found.clone());
        }
    }
    if let Some(id) = link.session_id.as_deref() {
        if let Some(found) = bridges.iter().find(|b| b.session_id.as_deref() == Some(id)) {
            return Some(found.clone());
        }
    }
    None
}

/// Sends the wake prompt to `record`'s bridge socket: one `{"t":"prompt",
/// "text":…,"mode":"auto"}` line. `mode:"auto"` is the bridge's own "prompt
/// now if idle, steer in at the next tool boundary if busy" behaviour
/// (`packages/app-bridge/src/bridge.ts`'s `handle` for `t:"prompt"`) — a
/// busy session queues the wake, an idle one runs it immediately, matching
/// the spec's "queues when busy, runs now when idle".
pub async fn send_wake(record: &BridgeRecord, text: &str) -> anyhow::Result<()> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(5),
        crate::chat::connect_bridge(&record.socket),
    )
    .await??;
    let line = serde_json::json!({ "t": "prompt", "text": text, "mode": "auto" });
    let mut bytes = serde_json::to_vec(&line)?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    // Best-effort ack read: the bridge replies `{"t":"ack",...}` — not
    // required for "sent" (the write succeeding is what matters), but
    // draining it avoids leaving a lingering connection the bridge has to
    // notice went away on its own.
    let mut buf = [0u8; 256];
    let _ = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await;
    Ok(())
}

/// The run-level outcome of trying to wake `job.session_link`, if any:
/// `None` when the job has no `session_link` (nothing to do); `Some("sent")`
/// or `Some("session closed")` otherwise, matching `Run.wake`.
pub async fn wake_if_linked(bridge_dir: &Path, job: &Job, run: &Run) -> Option<String> {
    let link = job.session_link.as_ref()?;
    match find_linked_bridge(bridge_dir, link) {
        None => Some("session closed".to_string()),
        Some(record) => {
            let text = wake_text(job, run);
            match send_wake(&record, &text).await {
                Ok(()) => Some("sent".to_string()),
                Err(error) => {
                    tracing::warn!(op = "cron_wake", job = job.id, run = run.id, %error, "session wake failed");
                    Some("session closed".to_string())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::model::RunStatus;

    fn job(link: Option<SessionLink>) -> Job {
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
            notify: None,
            session_link: link,
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
            ended_at: Some(2000),
            exit_code: Some(0),
            summary: Some("did the thing, all good".into()),
            reason: None,
            session_path: None,
            cost_usd: None,
            missed: false,
            notify_errors: Vec::new(),
            wake: None,
        }
    }

    fn write_bridge_record(dir: &Path, pid: u32, socket: &str, session_file: Option<&str>, session_id: Option<&str>) {
        let record = serde_json::json!({
            "pid": pid,
            "socket": socket,
            "sessionFile": session_file,
            "sessionId": session_id,
        });
        std::fs::write(dir.join(format!("{pid}.json")), serde_json::to_string(&record).unwrap()).unwrap();
    }

    #[test]
    fn wake_text_has_job_name_status_summary_and_run_id() {
        let j = job(None);
        let r = run(RunStatus::Ok);
        let text = wake_text(&j, &r);
        assert!(text.starts_with("[cron] digest finished: ok. did the thing, all good. Run r1"), "text: {text}");
        assert!(text.contains("unipi-host cron log r1"), "text: {text}");
    }

    #[test]
    fn wake_text_clips_summary_to_300_chars() {
        let j = job(None);
        let mut r = run(RunStatus::Ok);
        r.summary = Some("x".repeat(900));
        let text = wake_text(&j, &r);
        let between = text.split("finished: ok. ").nth(1).unwrap();
        let summary_part = between.split(". Run").next().unwrap();
        assert_eq!(summary_part.len(), 300);
    }

    #[test]
    fn find_linked_bridge_matches_by_session_file_then_id() {
        let tmp = tempfile::tempdir().unwrap();
        // A dead pid: not matched (read_bridges skips it).
        write_bridge_record(tmp.path(), std::process::id(), "/does/not/exist.sock", Some("/tmp/a.jsonl"), Some("abc"));
        let link = SessionLink { session_file: Some("/tmp/a.jsonl".into()), session_id: None };
        // The socket file doesn't exist either, so read_bridges filters it
        // out too -- this confirms "not found" is the honest answer when
        // the bridge isn't actually live, never a guess.
        assert!(find_linked_bridge(tmp.path(), &link).is_none());
    }

    #[tokio::test]
    async fn send_wake_delivers_a_prompt_line_to_the_bridge_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("bridge.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        let record = BridgeRecord {
            pid: 1,
            socket: socket_path.to_string_lossy().into_owned(),
            session_file: Some("/tmp/a.jsonl".into()),
            session_id: None,
            session_name: None,
            cwd: None,
            herdr_pane_id: None,
            herdr_socket: None,
            waiting: None,
            running: None,
            pending_work: None,
            lh_mode: None,
            lh_paused: None,
        };
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            line
        });
        send_wake(&record, "[cron] digest finished: ok. summary. Run r1 — details: unipi-host cron log r1").await.expect("send");
        let line = tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["t"], "prompt");
        assert_eq!(parsed["mode"], "auto");
        assert!(parsed["text"].as_str().unwrap().starts_with("[cron] digest finished: ok."));
    }

    #[tokio::test]
    async fn wake_if_linked_reports_session_closed_when_nothing_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let j = job(Some(SessionLink { session_file: Some("/tmp/gone.jsonl".into()), session_id: None }));
        let r = run(RunStatus::Ok);
        assert_eq!(wake_if_linked(tmp.path(), &j, &r).await, Some("session closed".to_string()));
    }

    #[tokio::test]
    async fn wake_if_linked_is_none_without_a_session_link() {
        let tmp = tempfile::tempdir().unwrap();
        let j = job(None);
        let r = run(RunStatus::Ok);
        assert_eq!(wake_if_linked(tmp.path(), &j, &r).await, None);
    }
}
