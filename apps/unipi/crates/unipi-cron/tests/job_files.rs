//! Job-file store tests (ext-abi §6.2): one file per job, the `jobs.json` → `jobs/*.json`
//! migration, unknown files ignored with diagnostics, read-modify-write under the lock.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use unipi_cron::model::Job;
use unipi_cron::paths::CronPaths;
use unipi_cron::store::{
    JOB_FILE_VERSION, insert_job, job_file_json, load_jobs, migrate_jobs_json, remove_job_file,
    save_job, scan_jobs, update_job,
};

fn paths() -> (tempfile::TempDir, CronPaths) {
    let tmp = tempfile::tempdir().unwrap();
    let p = CronPaths::new(tmp.path());
    (tmp, p)
}

fn job(id: &str) -> Job {
    Job::new(id, "test", "do it", "0 0 * * * *", "UTC", "/tmp")
}

#[test]
fn migration_splits_legacy_jobs_json_once() {
    let (_tmp, paths) = paths();
    let legacy = r#"[{
        "id": "digest", "name": "Digest", "prompt": "p", "schedule": "0 0 9 * * *",
        "tz": "UTC", "cwd": "/tmp", "time_limit_s": 600, "cost_cap_usd": 1.5,
        "created": 1, "updated": 2
    }]"#;
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(paths.legacy_jobs(), legacy).unwrap();
    let written = migrate_jobs_json(&paths).unwrap();
    assert_eq!(written, 1);
    assert!(!paths.legacy_jobs().exists());
    assert!(paths.root.join("jobs.json.imported").is_file());
    let jobs = load_jobs(&paths).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "digest");
    assert_eq!(jobs[0].time_limit_s, 600, "legacy snake_case loads");
    assert_eq!(jobs[0].cost_cap_usd, Some(1.5));
    // The split file is v2 camelCase.
    let raw = std::fs::read_to_string(paths.job_file("digest")).unwrap();
    assert!(raw.contains("\"v\": 2"), "{raw}");
    assert!(raw.contains("\"timeLimitS\": 600"), "{raw}");
    // Idempotent: opening again writes nothing.
    assert_eq!(migrate_jobs_json(&paths).unwrap(), 0);
    // A split file wins over a stale legacy copy.
    std::fs::write(paths.legacy_jobs(), legacy).unwrap();
    let mut edited = jobs.into_iter().next().unwrap();
    edited.name = "New".into();
    save_job(&paths, &edited).unwrap();
    assert_eq!(migrate_jobs_json(&paths).unwrap(), 0);
    assert_eq!(load_jobs(&paths).unwrap()[0].name, "New");
}

#[test]
fn scan_ignores_broken_files_with_diagnostics() {
    let (_tmp, paths) = paths();
    save_job(&paths, &job("good")).unwrap();
    std::fs::write(
        paths.job_file("mismatch"),
        job_file_json(&job("other")).to_string(),
    )
    .unwrap();
    std::fs::write(paths.job_file("broken"), "{ not json").unwrap();
    let old = serde_json::json!({
        "v": 1, "id": "old", "name": "n", "prompt": "p",
        "schedule": "0 0 * * * *", "tz": "UTC", "cwd": "/tmp", "created": 0, "updated": 0
    });
    std::fs::write(paths.job_file("old"), old.to_string()).unwrap();
    std::fs::write(paths.jobs_dir().join("notes.txt"), "ignore me").unwrap();
    let (jobs, diagnostics) = scan_jobs(&paths).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "good");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("id \"other\" \u{2260} file name")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains(&format!("schema version 1 \u{2260} {JOB_FILE_VERSION}"))),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|d| d.contains("broken.json")),
        "{diagnostics:?}"
    );
}

#[test]
fn update_job_applies_or_aborts_under_the_lock() {
    let (_tmp, paths) = paths();
    save_job(&paths, &job("a")).unwrap();
    let ((), updated) = update_job(&paths, "a", |job| {
        job.name = "Renamed".into();
        Ok::<(), String>(())
    })
    .unwrap()
    .expect("read ok")
    .expect("applied");
    assert_eq!(updated.name, "Renamed");
    let aborted = update_job(&paths, "a", |_job: &mut Job| {
        Err::<(), String>("nope".to_owned())
    })
    .unwrap()
    .expect_err("closure error surfaces");
    assert_eq!(aborted, "nope");
    assert_eq!(load_jobs(&paths).unwrap()[0].name, "Renamed");
    let missing = update_job(&paths, "missing", |_job: &mut Job| Ok(()))
        .unwrap()
        .expect("read ok");
    assert!(missing.is_none());
}

#[test]
fn insert_rejects_taken_ids_and_delete_removes_the_file() {
    let (_tmp, paths) = paths();
    insert_job(&paths, &job("a")).unwrap();
    assert!(
        insert_job(&paths, &job("a"))
            .unwrap_err()
            .contains("already exists")
    );
    assert!(remove_job_file(&paths, "a").unwrap());
    assert!(!remove_job_file(&paths, "a").unwrap());
    assert!(load_jobs(&paths).unwrap().is_empty(), "job file removed");
}
