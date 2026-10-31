//! The embedded single-owner scheduler (ext-abi §6.3): owner lock exclusivity, startup
//! migration, and the `jobs/` watcher — a job file written *after* the owner started is
//! picked up without a restart and fires.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::time::Duration;

use unipi_cron::CronError;
use unipi_cron::embedded::Embedded;
use unipi_cron::model::Job;
use unipi_cron::paths::CronPaths;
use unipi_cron::store::{load_jobs, save_job};

fn due_soon_job(id: &str, cwd: &str) -> Job {
    let at = chrono::Utc::now() + chrono::Duration::seconds(2);
    let mut job = Job::new(
        id,
        "watcher test",
        "say hi",
        &format!("at:{}", at.to_rfc3339()),
        "UTC",
        cwd,
    );
    let now = u64::try_from(chrono::Utc::now().timestamp_millis().max(0)).unwrap_or(0);
    job.created = now;
    job.updated = now;
    job
}

#[tokio::test]
async fn embedded_owner_is_exclusive_and_fires_edits_made_after_start() {
    // The fire decision (and its run record) is observable without a real `oino` binary:
    // the record is written before the spawn (and marked failed when none resolves).
    let tmp = tempfile::tempdir().unwrap();
    let paths = CronPaths::new(tmp.path());
    std::fs::create_dir_all(paths.jobs_dir()).unwrap();
    save_job(
        &paths,
        &due_soon_job("existing", tmp.path().to_str().unwrap()),
    )
    .unwrap();

    // A pre-v2 `jobs.json` is migrated at startup.
    std::fs::write(
        paths.legacy_jobs(),
        r#"[{"id":"legacy","name":"L","prompt":"p","schedule":"0 0 9 * * *","tz":"UTC","cwd":"/tmp","created":1,"updated":1}]"#,
    )
    .unwrap();

    let mut scheduler = unipi_cron::Scheduler::with_paths(paths.clone());
    // Long enough that only the jobs/ watcher (or the startup scan) can notice the new
    // file within the test's window — the regular tick alone would be too slow.
    scheduler.tick_interval = Duration::from_secs(30);
    let embedded = Embedded::start_with(scheduler).unwrap();
    assert_eq!(
        load_jobs(&paths)
            .unwrap()
            .iter()
            .map(|j| j.id.as_str())
            .collect::<Vec<_>>(),
        vec!["existing", "legacy"],
        "startup migrated the legacy list"
    );

    // The scheduler.lock makes the owner exclusive.
    let second = Embedded::start(paths.clone());
    assert!(
        matches!(second, Err(CronError::AlreadyOwned(_))),
        "a second owner must fail fast"
    );

    // A job written after the owner started. Its fire time is ~2s out; once it is due,
    // rewriting the file (a plain edit) is what the watcher must notice — the regular
    // tick is 30s away, so only the watcher's rescan can fire it in this window.
    let late = due_soon_job("late", tmp.path().to_str().unwrap());
    save_job(&paths, &late).unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await; // past the fire time
    let mut edited = late.clone();
    edited.name = "edited".into();
    save_job(&paths, &edited).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let ran = std::fs::read_dir(paths.job_runs("late"))
            .is_ok_and(|mut entries| entries.next().is_some());
        if ran {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the watcher never fired the job written after start; cron tree: {:?}",
            tree(&paths.root)
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    embedded.shutdown();
}

/// Every path under `root` (used in the failure message only).
fn tree(root: &std::path::Path) -> String {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(
                path.strip_prefix(dir.parent().unwrap_or(dir))
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
            if path.is_dir() {
                walk(&path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.join(", ")
}
