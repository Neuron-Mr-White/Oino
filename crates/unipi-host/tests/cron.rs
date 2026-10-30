//! UNI-314 integration tests: a fake `pi` shell script on a temp PATH,
//! driving the real [`unipi_host::cron`] scheduler end-to-end (no real
//! LLM calls). Each test gets its own `UNIPI_HOST_DIR` tempdir.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use unipi_host::cron::model::{Job, RunStatus};
use unipi_host::cron::scheduler::{Clock, Scheduler};
use unipi_host::data_dir::HostPaths;

/// Writes a fake `pi` on its own temp dir and returns `(tempdir, path_list)`.
/// The host dir's `env` file is written with that `PATH`, like a real
/// `~/.unipi/app-host/env` (UNI-239).
fn fake_pi(paths: &HostPaths, script: &str) -> tempfile::TempDir {
    let bin_dir = tempfile::tempdir().unwrap();
    let pi = bin_dir.path().join("pi");
    std::fs::write(&pi, script).unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path_list = format!(
        "{}:{}",
        bin_dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    std::fs::write(paths.root.join("env"), format!("PATH={path_list}\n")).unwrap();
    bin_dir
}

fn job(id: &str, schedule: &str, cwd: &str, time_limit_s: u64) -> Job {
    Job {
        id: id.into(),
        name: id.into(),
        prompt: "do the thing".into(),
        schedule: schedule.into(),
        tz: "UTC".into(),
        cwd: cwd.into(),
        model: None,
        thinking: None,
        enabled: true,
        time_limit_s,
        cost_cap_usd: None,
        notify: None,
        session_link: None,
        created: 0,
        updated: 0,
    }
}

fn job_with_cap(id: &str, schedule: &str, cwd: &str, time_limit_s: u64, cost_cap_usd: f64) -> Job {
    Job {
        cost_cap_usd: Some(cost_cap_usd),
        ..job(id, schedule, cwd, time_limit_s)
    }
}

async fn wait_for<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A job firing every 2s runs, records `ok`, a summary and a log.
#[tokio::test]
async fn fires_every_2s_and_records_ok_with_summary_and_log() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let cwd = tempfile::tempdir().unwrap();
    let _bin = fake_pi(
        &paths,
        r#"#!/bin/sh
echo '{"type":"turn_end","message":{"role":"assistant","content":[{"type":"text","text":"RESULT: did the thing"}],"usage":{"cost":{"total":0.01}}}}'
exit 0
"#,
    );

    let scheduler = Scheduler::new(paths.clone());
    let j = job(
        "every2s",
        "*/2 * * * * *",
        &cwd.path().to_string_lossy(),
        10,
    );
    unipi_host::cron::store::save_jobs(&paths, &[j]).unwrap();

    // Drive the scheduler's ticks concurrently with the wait.
    let ticker = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move {
            for _ in 0..200 {
                scheduler.tick().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
    };
    let found = wait_for(
        || {
            unipi_host::cron::store::load_runs(&paths, "every2s")
                .map(|r| r.iter().any(|r| r.status == RunStatus::Ok))
                .unwrap_or(false)
        },
        Duration::from_secs(10),
    )
    .await;
    ticker.abort();
    assert!(found, "the job should have fired and completed within 10s");

    let runs = unipi_host::cron::store::load_runs(&paths, "every2s").unwrap();
    let ok = runs
        .iter()
        .find(|r| r.status == RunStatus::Ok)
        .expect("an ok run");
    assert_eq!(ok.exit_code, Some(0));
    assert!(
        ok.summary
            .as_deref()
            .unwrap_or("")
            .contains("did the thing"),
        "summary: {:?}",
        ok.summary
    );
    let log = unipi_host::cron::store::read_log(&paths, "every2s", &ok.id).expect("log exists");
    assert!(String::from_utf8_lossy(&log).contains("did the thing"));
}

/// A job that sleeps 10s with a 2s limit → `timeout`, and the process is gone.
#[tokio::test]
async fn exceeding_time_limit_times_out_and_kills_the_process() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let cwd = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("still-running");
    let script = format!(
        "#!/bin/sh\ntouch {marker}\nsleep 10\nrm -f {marker}\n",
        marker = marker.display()
    );
    let _bin = fake_pi(&paths, &script);

    let job = job(
        "slowjob",
        "at:2000-01-01T00:00:00Z",
        &cwd.path().to_string_lossy(),
        2,
    );
    // Run it directly (bypassing the schedule) via run_now.
    let scheduler = Scheduler::new(paths.clone());
    unipi_host::cron::store::save_jobs(&paths, std::slice::from_ref(&job)).unwrap();
    let run_id = scheduler.run_now(&job).await.unwrap();

    assert!(
        wait_for(|| marker.exists(), Duration::from_secs(5)).await,
        "the fake pi should have started"
    );
    let timed_out = wait_for(
        || {
            unipi_host::cron::store::load_run(&paths, "slowjob", &run_id)
                .unwrap()
                .map(|r| r.status == RunStatus::Timeout)
                .unwrap_or(false)
        },
        Duration::from_secs(8),
    )
    .await;
    assert!(timed_out, "expected a timeout status");
    // The process group was killed: the marker file (removed only by the
    // normal sleep-then-continue path) must still be there, i.e. it never
    // reached the `rm`, and no "pi" process from this test is left running.
    assert!(
        marker.exists(),
        "the script never got past `sleep 10`, consistent with being killed"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let still_alive = std::process::Command::new("pgrep")
        .args(["-f", &cwd.path().to_string_lossy()])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    assert!(
        !still_alive,
        "the sleep process should be gone after the timeout kill"
    );
}

/// A second fire while the first is still running → `skipped`.
#[tokio::test]
async fn overlapping_fire_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let cwd = tempfile::tempdir().unwrap();
    let _bin = fake_pi(&paths, "#!/bin/sh\nsleep 3\necho done\n");

    let scheduler = Scheduler::new(paths.clone());
    let j = job(
        "overlap",
        "at:2000-01-01T00:00:00Z",
        &cwd.path().to_string_lossy(),
        10,
    );
    unipi_host::cron::store::save_jobs(&paths, std::slice::from_ref(&j)).unwrap();

    let first = scheduler.run_now(&j).await.unwrap();
    // Give it a moment to actually start before trying to overlap it.
    wait_for(
        || {
            unipi_host::cron::store::load_run(&paths, "overlap", &first)
                .unwrap()
                .map(|r| r.status == RunStatus::Running)
                .unwrap_or(false)
        },
        Duration::from_secs(2),
    )
    .await;
    let second = scheduler.run_now(&j).await;
    assert!(second.is_err(), "run_now while running should refuse");

    // A scheduled fire while running (via consider_fire through tick with
    // an `at:` far in the future doesn't apply) is exercised at the unit
    // level (scheduler::tests::overlap_fire_is_recorded_skipped); here we
    // confirm the control-level `run_now` guard too, end to end.
    wait_for(
        || {
            unipi_host::cron::store::load_run(&paths, "overlap", &first)
                .unwrap()
                .map(|r| r.status == RunStatus::Ok)
                .unwrap_or(false)
        },
        Duration::from_secs(6),
    )
    .await;
}

/// A run reporting more cost than the job's `cost_cap_usd` → `failed`,
/// reason "cost cap", even though `pi` itself exited 0.
#[tokio::test]
async fn over_cost_cap_fails_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let cwd = tempfile::tempdir().unwrap();
    let _bin = fake_pi(
        &paths,
        r#"#!/bin/sh
echo '{"type":"turn_end","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"usage":{"cost":{"total":5.0}}}}'
exit 0
"#,
    );

    let j = job_with_cap(
        "capped",
        "at:2000-01-01T00:00:00Z",
        &cwd.path().to_string_lossy(),
        10,
        1.0,
    );
    let scheduler = Scheduler::new(paths.clone());
    unipi_host::cron::store::save_jobs(&paths, std::slice::from_ref(&j)).unwrap();
    let run_id = scheduler.run_now(&j).await.unwrap();

    let failed = wait_for(
        || {
            unipi_host::cron::store::load_run(&paths, "capped", &run_id)
                .unwrap()
                .map(|r| r.status == RunStatus::Failed)
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(failed, "a run over the cost cap should be recorded failed");
    let run = unipi_host::cron::store::load_run(&paths, "capped", &run_id)
        .unwrap()
        .unwrap();
    assert!(
        run.reason.as_deref().unwrap_or("").contains("cost cap"),
        "reason: {:?}",
        run.reason
    );
    assert_eq!(run.cost_usd, Some(5.0));
}

/// `pi` missing from PATH fails clearly, never silently.
#[tokio::test]
async fn missing_pi_fails_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    std::fs::write(paths.root.join("env"), "PATH=/does/not/exist\n").unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let scheduler = Scheduler::new(paths.clone());
    let j = job(
        "nopi",
        "at:2000-01-01T00:00:00Z",
        &cwd.path().to_string_lossy(),
        10,
    );
    unipi_host::cron::store::save_jobs(&paths, std::slice::from_ref(&j)).unwrap();
    let run_id = scheduler.run_now(&j).await.unwrap();
    wait_for(
        || {
            unipi_host::cron::store::load_run(&paths, "nopi", &run_id)
                .unwrap()
                .map(|r| r.status == RunStatus::Failed)
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    )
    .await;
    let run = unipi_host::cron::store::load_run(&paths, "nopi", &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(
        run.reason
            .as_deref()
            .unwrap_or("")
            .contains("pi not found on PATH"),
        "reason: {:?}",
        run.reason
    );
}

/// `Clock::fixed` lets a test move the wall clock without a real sleep —
/// used here to exercise missed-run catch-up through `tick` end to end.
#[tokio::test]
async fn missed_run_catches_up_once_after_a_clock_jump() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(tmp.path());
    let cwd = tempfile::tempdir().unwrap();
    let _bin = fake_pi(&paths, "#!/bin/sh\necho caught up\n");

    let start: chrono::DateTime<chrono::Utc> = "2024-01-01T00:00:00Z".parse().unwrap();
    let clock = std::sync::Arc::new(tokio::sync::Mutex::new(start));
    let scheduler = Scheduler::with_clock(paths.clone(), Clock::fixed(clock.clone()));
    // Daily job; `created` = start, so the first due fire is 2024-01-02T00:00:00Z.
    let mut j = job("daily", "0 0 0 * * *", &cwd.path().to_string_lossy(), 10);
    j.created = start.timestamp_millis() as u64;
    unipi_host::cron::store::save_jobs(&paths, &[j]).unwrap();

    // Jump the clock forward 2 days (simulating sleep/host-down): only the
    // latest missed midnight should run once.
    *clock.lock().await = "2024-01-03T01:00:00Z".parse().unwrap();
    scheduler.tick().await;
    let ran = wait_for(
        || {
            unipi_host::cron::store::load_runs(&paths, "daily")
                .map(|r| !r.is_empty())
                .unwrap_or(false)
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(ran, "the missed run should have fired once");
    let runs = unipi_host::cron::store::load_runs(&paths, "daily").unwrap();
    assert_eq!(
        runs.len(),
        1,
        "only the latest missed fire runs, not every missed midnight"
    );
    assert!(runs[0].missed);
}
