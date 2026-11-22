//! `list --brief`: the board list row the app's 1 MiB control frame can carry.
//! Full body stays; activity keeps the last 48 h, the last 14 days of status
//! transitions and the transition into the current status; the report is cut
//! at 600 chars. Plain `list` keeps returning everything.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs,
    unreachable_pub,
    clippy::too_many_lines,
    clippy::doc_markdown,
    reason = "integration tests: helpers outside #[test] fns may panic on fixture failures"
)]

mod common;

use chrono::{Duration, SecondsFormat, Utc};
use common::Fixture;
use serde_json::Value;

const PREFIX: &str = "the long recent note";

fn stamp(delta: Duration) -> String {
    (Utc::now() - delta).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn list(fixture: &Fixture, args: &[&str]) -> Value {
    let output = common::cli(fixture, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("list json")
}

fn texts(task: &Value) -> Vec<String> {
    task["activity"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap().to_string())
        .collect()
}

fn brief_row(fixture: &Fixture) -> Value {
    let json = list(fixture, &["list", "--all", "--brief", "--json"]);
    json["tasks"].as_array().unwrap()[0].clone()
}

#[test]
fn brief_drops_old_notes_keeps_recent_and_the_current_transition() {
    let fixture = Fixture::new();
    let long = format!("{PREFIX} {}", "x".repeat(600));
    common::write_task_file(
        &fixture,
        "FIX-910",
        &format!(
            "---\nid: FIX-910\ntitle: Brief trim\nstatus: todo\npriority: none\norder: 1000\ndeps: []\nlabels: []\ncreated: {created}\nupdated: {updated}\nrun:\n---\n\nBody text.\n\n## Activity\n- {created} [user] created in todo\n- {old} [user] a plain old note\n- {transition} [agent] moved backlog → done: shipped\n- {recent} [user] a recent note\n- {latest} [agent] {long}\n",
            created = stamp(Duration::days(40)),
            updated = stamp(Duration::hours(1)),
            old = stamp(Duration::days(20)),
            transition = stamp(Duration::days(10)),
            recent = stamp(Duration::hours(47)),
            latest = stamp(Duration::hours(1)),
        ),
    );

    let row = brief_row(&fixture);
    let kept = texts(&row);
    // `created in todo` is the last transition into the current status, so it
    // survives at any age; the old plain note is dropped; the 10-day status
    // transition and the 47 h note stay; the 1 h note is cut at 500 chars.
    assert_eq!(kept.len(), 4, "{kept:?}");
    assert_eq!(kept[0], "created in todo");
    assert_eq!(kept[1], "moved backlog → done: shipped");
    assert_eq!(kept[2], "a recent note");
    assert!(kept[3].starts_with(PREFIX), "{:?}", kept[3]);
    assert!(kept[3].ends_with('…'), "{:?}", kept[3]);
    assert_eq!(kept[3].chars().count(), 501, "500 chars + the ellipsis");
    assert_eq!(row["body"], "Body text.");
    assert_eq!(row["activityTrimmed"], true);
    assert!(row.get("statusReportTrimmed").is_none());
}

#[test]
fn plain_list_is_unchanged() {
    let fixture = Fixture::new();
    let long = format!("{PREFIX} {}", "x".repeat(600));
    common::write_task_file(
        &fixture,
        "FIX-911",
        &format!(
            "---\nid: FIX-911\ntitle: Plain trim\nstatus: todo\npriority: none\norder: 1000\ndeps: []\nlabels: []\ncreated: {created}\nupdated: {updated}\nrun:\n---\n\nBody text.\n\n## Activity\n- {created} [user] created in todo\n- {old} [user] a plain old note\n- {latest} [agent] {long}\n",
            created = stamp(Duration::days(40)),
            updated = stamp(Duration::hours(1)),
            old = stamp(Duration::days(20)),
            latest = stamp(Duration::hours(1)),
        ),
    );

    let row = list(&fixture, &["list", "--all", "--json"])["tasks"]
        .as_array()
        .unwrap()[0]
        .clone();
    let kept = texts(&row);
    assert_eq!(kept.len(), 3, "{kept:?}");
    assert_eq!(kept[1], "a plain old note");
    assert_eq!(kept[2], long);
    assert!(row.get("activityTrimmed").is_none());
    assert!(row.get("statusReportTrimmed").is_none());
}

#[test]
fn brief_trims_the_status_report_and_keeps_its_transition() {
    let fixture = Fixture::new();
    let report = format!("the full report {}", "y".repeat(700));
    common::write_task_file(
        &fixture,
        "FIX-912",
        &format!(
            "---\nid: FIX-912\ntitle: Brief report\nstatus: in_review\npriority: none\norder: 1000\ndeps: []\nlabels: []\ncreated: {created}\nupdated: {updated}\nrun:\n---\n\nBody text.\n\n## Activity\n- {created} [user] created in todo\n- {finished} [agent] finished: {report}\n",
            created = stamp(Duration::days(40)),
            updated = stamp(Duration::days(30)),
            finished = stamp(Duration::days(30)),
        ),
    );

    let row = brief_row(&fixture);
    let kept = texts(&row);
    // The finished entry is the transition into `in_review` (kept at any age)
    // and is cut as activity; the report itself keeps its own 600 chars.
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(
        kept[0].starts_with("finished: the full report"),
        "{:?}",
        kept[0]
    );
    assert_eq!(kept[0].chars().count(), 201, "200 chars + the ellipsis");
    let text = row["statusReport"]["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), 601, "600 chars + the ellipsis");
    assert!(text.starts_with("the full report"), "{text:?}");
    assert_eq!(row["statusReportTrimmed"], true);
    assert_eq!(row["activityTrimmed"], true);
}

#[test]
fn brief_keeps_an_old_undo_anchor_into_the_current_status() {
    let fixture = Fixture::new();
    common::write_task_file(
        &fixture,
        "FIX-913",
        &format!(
            "---\nid: FIX-913\ntitle: Undo anchor\nstatus: in_review\npriority: none\norder: 1000\ndeps: []\nlabels: []\ncreated: {created}\nupdated: {undo}\nrun:\n---\n\nBody text.\n\n## Activity\n- {created} [user] created in todo\n- {finished} [agent] finished: first review\n- {rework} [agent] rework: needs another pass\n- {undo} [user] undo: todo → in_review (state restored)\n",
            created = stamp(Duration::days(40)),
            finished = stamp(Duration::days(30)),
            rework = stamp(Duration::days(20)),
            undo = stamp(Duration::days(18)),
        ),
    );

    let row = brief_row(&fixture);
    let kept = texts(&row);
    // The 18-day-old undo is the transition into the current status: the lane
    // entered is its LAST `→` segment, so it and the real transition before it
    // survive the 14-day window; the 20-day rework does not.
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert_eq!(kept[0], "finished: first review");
    assert_eq!(kept[1], "undo: todo → in_review (state restored)");
}

#[test]
fn brief_keeps_an_old_review_enter_when_its_exit_is_recent() {
    let fixture = Fixture::new();
    common::write_task_file(
        &fixture,
        "FIX-914",
        &format!(
            "---\nid: FIX-914\ntitle: Old review cycle\nstatus: done\npriority: none\norder: 1000\ndeps: []\nlabels: []\ncreated: {created}\nupdated: {exit}\nrun:\n---\n\nBody text.\n\n## Activity\n- {created} [user] created in todo\n- {enter} [agent] finished: reviewed after twenty days\n- {exit} [user] moved in_review → done: shipped\n",
            created = stamp(Duration::days(40)),
            enter = stamp(Duration::days(20)),
            exit = stamp(Duration::days(10)),
        ),
    );

    let row = brief_row(&fixture);
    let kept = texts(&row);
    // The enter is 20 days old, but its exit is inside the 14-day window, so
    // the dashboard can still pair the review cycle from the brief slice.
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert!(kept[0].starts_with("finished: reviewed"), "{:?}", kept[0]);
    assert_eq!(kept[1], "moved in_review → done: shipped");
}
