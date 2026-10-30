//! UNI-316 integration tests: session wake against a fake bridge unix
//! socket (never a real pi session).

#![cfg(unix)]

use unipi_host::cron::model::{Job, Run, RunStatus, SessionLink};
use unipi_host::cron::wake::{wake_if_linked, wake_text};

fn job(link: Option<SessionLink>) -> Job {
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
        notify: None,
        session_link: link,
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
        ended_at: Some(2000),
        exit_code: Some(0),
        summary: Some("Summarized 12 emails, all caught up".into()),
        reason: None,
        session_path: None,
        cost_usd: None,
        missed: false,
        notify_errors: Vec::new(),
        wake: None,
    }
}

/// Writes a `~/.unipi/bridge/<pid>.json`-shaped record pointing at a real
/// listening unix socket, using *this* test process's own pid (so
/// `pid_alive` in `chat::read_bridges` finds it alive — never a real pi).
fn write_bridge_record(dir: &std::path::Path, socket: &std::path::Path, session_file: Option<&str>, session_id: Option<&str>) {
    let pid = std::process::id();
    let record = serde_json::json!({
        "pid": pid,
        "socket": socket.to_string_lossy(),
        "sessionFile": session_file,
        "sessionId": session_id,
    });
    std::fs::write(dir.join(format!("{pid}.json")), serde_json::to_string(&record).unwrap()).unwrap();
}

/// A fake bridge: binds a unix socket, accepts one connection, returns the
/// line it received.
async fn fake_bridge(dir: &std::path::Path) -> (std::path::PathBuf, tokio::sync::oneshot::Receiver<String>) {
    let socket_path = dir.join("bridge.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let _ = tx.send(line);
    });
    (socket_path, rx)
}

/// A live bridge matching by `session_file` receives the wake prompt
/// (`{t:"prompt",mode:"auto"}`), and `wake_if_linked` reports "sent".
#[tokio::test]
async fn a_live_bridge_receives_the_wake_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let (socket_path, rx) = fake_bridge(tmp.path()).await;
    write_bridge_record(tmp.path(), &socket_path, Some("/tmp/project/session.jsonl"), None);

    let j = job(Some(SessionLink { session_file: Some("/tmp/project/session.jsonl".into()), session_id: None }));
    let r = run(RunStatus::Ok);
    let outcome = wake_if_linked(tmp.path(), &j, &r).await;
    assert_eq!(outcome, Some("sent".to_string()));

    let line = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.unwrap().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(parsed["t"], "prompt");
    assert_eq!(parsed["mode"], "auto");
    let text = parsed["text"].as_str().unwrap();
    assert_eq!(text, wake_text(&j, &r));
    assert!(text.contains("daily digest finished: ok"), "text: {text}");
    assert!(text.contains("Summarized 12 emails"), "text: {text}");
    assert!(text.contains("unipi-host cron log r1"), "text: {text}");
}

/// Matching by `session_id` when no `session_file` is given.
#[tokio::test]
async fn matches_by_session_id_when_no_session_file_is_set() {
    let tmp = tempfile::tempdir().unwrap();
    let (socket_path, rx) = fake_bridge(tmp.path()).await;
    write_bridge_record(tmp.path(), &socket_path, Some("/tmp/other.jsonl"), Some("sess-abc"));

    let j = job(Some(SessionLink { session_file: None, session_id: Some("sess-abc".into()) }));
    let r = run(RunStatus::Failed);
    let outcome = wake_if_linked(tmp.path(), &j, &r).await;
    assert_eq!(outcome, Some("sent".to_string()));
    let line = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.unwrap().unwrap();
    assert!(line.contains("\"t\":\"prompt\""));
}

/// No bridge at all (the session closed): recorded `"session closed"`,
/// never an error, never a panic.
#[tokio::test]
async fn a_closed_session_is_recorded_not_sent() {
    let tmp = tempfile::tempdir().unwrap(); // empty bridge dir
    let j = job(Some(SessionLink { session_file: Some("/tmp/gone.jsonl".into()), session_id: None }));
    let r = run(RunStatus::Ok);
    let outcome = wake_if_linked(tmp.path(), &j, &r).await;
    assert_eq!(outcome, Some("session closed".to_string()));
}

/// A job with no `session_link` at all: nothing to do, `Run.wake` stays
/// unset (`None`).
#[tokio::test]
async fn no_session_link_means_no_wake_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let j = job(None);
    let r = run(RunStatus::Ok);
    assert_eq!(wake_if_linked(tmp.path(), &j, &r).await, None);
}
