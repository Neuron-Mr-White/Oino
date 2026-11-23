//! Restart-safe run ownership and daemon cleanup, using only private fixture processes.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "test assertions; one linear restart-safe ownership scenario"
)]
use std::time::{Duration, Instant};
use unipi_cron::{store, supervisor, CronPaths, Job, Run, RunStatus, Scheduler};

async fn until(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

async fn scenario(stop: bool, daemon: bool, adopt: bool) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let paths = CronPaths::new(tmp.path());
    paths.ensure().unwrap();
    let job = Job {
        time_limit_s: if stop { 60 } else { 2 },
        ..Job::new(
            "j",
            "Restart test",
            "p",
            "0 0 1 * * *",
            "UTC",
            tmp.path().to_str().unwrap(),
        )
    };
    store::save_job(&paths, &job).unwrap();
    let now = chrono::Utc::now().timestamp_millis().cast_unsigned();
    let run: Run = serde_json::from_value(serde_json::json!({"id":"r", "job_id":"j", "status":"running", "fired_at":now, "started_at":now})).unwrap();
    store::save_run(&paths, &run).unwrap();
    let script = tmp.path().join("mock.py");
    std::fs::write(
        &script,
        format!(
            r"#!/usr/bin/python3
import os, signal, time
signal.signal(signal.SIGINT, signal.SIG_IGN)
if {daemon}:
    pid = os.fork()
    if pid == 0:
        os.setsid()
        if os.fork(): os._exit(0)
        open('daemon.pid', 'w').write(str(os.getpid()))
        while True: time.sleep(1)
while True: time.sleep(1)
",
            daemon = if daemon { "True" } else { "False" }
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_unipi-host"))
        .args(["cron-supervisor", tmp.path().to_str().unwrap(), "j", "r"])
        .env("OINO_CRON_BIN", &script)
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    until(|| {
        store::load_run(&paths, "j", "r")
            .unwrap()
            .is_some_and(|r| supervisor::live(&r))
    })
    .await;
    if daemon {
        until(|| tmp.path().join("daemon.pid").exists()).await;
    }
    // A completely new scheduler has no in-memory reservation. It must adopt this run.
    let scheduler = Scheduler::with_paths(paths.clone());
    if adopt {
        scheduler.reconcile().await.unwrap();
        assert_eq!(scheduler.running_job_ids().await, ["j"]);
    }
    let before = Instant::now();
    if stop {
        assert!(scheduler.stop("r").await);
        assert!(scheduler.stop("r").await);
    }
    until(|| {
        store::load_run(&paths, "j", "r")
            .unwrap()
            .unwrap()
            .status
            .is_finished()
    })
    .await;
    let finished = store::load_run(&paths, "j", "r").unwrap().unwrap();
    assert_eq!(
        finished.status,
        if stop {
            RunStatus::Stopped
        } else {
            RunStatus::Timeout
        }
    );
    if stop {
        assert_eq!(finished.reason.as_deref(), Some("Stopped by you"));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !scheduler.running_job_ids().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let _ = child.wait().await.unwrap();
    if daemon {
        let pid: u32 = std::fs::read_to_string(tmp.path().join("daemon.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            supervisor::identity(pid).is_none(),
            "detached daemon survived"
        );
    }
    eprintln!(
        "adopt stop={stop} daemon={daemon}: {}ms {:?}",
        before.elapsed().as_millis(),
        finished.status
    );
}

#[tokio::test]
async fn adopted_stop_reaps_double_forked_setsid_daemon_after_grace() {
    scenario(true, true, true).await;
}
#[tokio::test]
async fn adopted_run_keeps_original_time_limit() {
    scenario(false, true, true).await;
}

#[tokio::test]
async fn supervisor_enforces_deadline_without_any_host_or_scheduler() {
    scenario(false, true, false).await;
}

#[tokio::test]
async fn supervisor_records_normal_completion_without_a_host() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = CronPaths::new(tmp.path());
    paths.ensure().unwrap();
    let job = Job::new(
        "j",
        "j",
        "p",
        "0 0 1 * * *",
        "UTC",
        tmp.path().to_str().unwrap(),
    );
    store::save_job(&paths, &job).unwrap();
    let run: Run = serde_json::from_value(serde_json::json!({"id":"r","job_id":"j","status":"running","fired_at":0,"started_at":chrono::Utc::now().timestamp_millis()})).unwrap();
    store::save_run(&paths, &run).unwrap();
    let status = tokio::process::Command::new(env!("CARGO_BIN_EXE_unipi-host"))
        .args(["cron-supervisor", tmp.path().to_str().unwrap(), "j", "r"])
        .env("OINO_CRON_BIN", "/bin/true")
        .process_group(0)
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let finished = store::load_run(&paths, "j", "r").unwrap().unwrap();
    assert_eq!(finished.status, RunStatus::Ok);
    assert_eq!(finished.exit_code, Some(0));
    assert!(finished.ended_at.is_some());
}

#[tokio::test]
async fn reconcile_rejects_pid_reuse_and_closes_missing_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = CronPaths::new(tmp.path());
    paths.ensure().unwrap();
    let job = Job::new(
        "j",
        "j",
        "p",
        "0 0 1 * * *",
        "UTC",
        tmp.path().to_str().unwrap(),
    );
    store::save_job(&paths, &job).unwrap();
    for (id, identity) in [
        (
            "reused",
            serde_json::json!({"boot":"wrong boot", "start":"0"}),
        ),
        ("missing", serde_json::Value::Null),
    ] {
        let run: Run = serde_json::from_value(serde_json::json!({"id":id,"job_id":"j","status":"running","fired_at":0,"pid":std::process::id(),"process_identity":identity})).unwrap();
        store::save_run(&paths, &run).unwrap();
    }
    let scheduler = Scheduler::with_paths(paths.clone());
    scheduler.reconcile().await.unwrap();
    assert_eq!(scheduler.running_job_ids().await.len(), 0);
    for run in store::load_all_runs(&paths).unwrap() {
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.reason.unwrap().contains("while the app was closed"));
    }
    assert!(supervisor::identity(std::process::id()).is_some());
}
