//! Task file format: round-trip, strict parsing with line numbers, `--fix`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs,
    unreachable_pub,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::format_push_string,
    clippy::used_underscore_binding,
    clippy::needless_continue,
    reason = "integration tests: helpers outside #[test] fns may panic on fixture failures"
)]

mod common;

use chrono::{TimeZone, Utc};
use common::{Fixture, VALID_TASK};
use oino_kanboard::commands;
use oino_kanboard::error::Problem;
use oino_kanboard::format;
use oino_kanboard::model::{Actor, Priority, Run, RunMode, Status, Task};

fn sample() -> Task {
    let mut task = Task::new(
        "FIX-12".into(),
        "Add --verbose flag: quoting, commas, []".into(),
        Status::InProgress,
        Priority::High,
        3000,
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap(),
    );
    task.deps = vec!["FIX-10".into(), "FIX-11".into()];
    task.labels = vec!["cli".into(), "polish".into()];
    task.body = "Description with **markdown**.\n\n## Notes\n- one\n- two".into();
    task.run = Some(Run {
        session: "01a0ceb8".into(),
        pid: 12345,
        host: "coffee".into(),
        mode: RunMode::Goal,
        goal: Some("goal-7".into()),
        started: Utc.with_ymd_and_hms(2026, 9, 24, 10, 5, 0).unwrap(),
        owner: oino_kanboard::model::RunOwner::Agent,
    });
    task.push_activity(
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 5, 0).unwrap(),
        Actor::System,
        "claimed by session 01a0ceb8 (mode goal)",
    );
    task.push_activity(
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 20, 0).unwrap(),
        Actor::Agent,
        "blocked: which log format do you want?\nsecond line of the note",
    );
    task
}

#[test]
fn render_parse_round_trip_is_lossless() {
    let task = sample();
    let text = format::render(&task);
    let (parsed, problems) = format::parse("FIX-12.md", &text);
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    let parsed = parsed.expect("task");
    assert_eq!(parsed, task);
    // Re-rendering is byte-identical (canonical form).
    assert_eq!(format::render(&parsed), text);
}

#[test]
fn renders_the_spec_example_shape() {
    let mut task = Task::new(
        "UNI-12".into(),
        "Add --verbose flag to loop.sh".into(),
        Status::Todo,
        Priority::None,
        3000,
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap(),
    );
    task.body = "Free-form description / acceptance criteria (markdown).".into();
    let text = format::render(&task);
    assert!(text.contains("id: UNI-12\n"));
    assert!(text.contains("status: todo\n"));
    assert!(text.contains("priority: none\n"));
    assert!(text.contains("order: 3000\n"));
    assert!(text.contains("deps: []\n"));
    assert!(text.contains("run:\n"));
    assert!(text.contains("## Activity\n"));
}

#[test]
fn multi_line_activity_uses_indented_continuation() {
    let task = sample();
    let text = format::render(&task);
    assert!(text.contains("- 2026-09-24T10:20:00Z [agent] blocked: which log format do you want?\n  second line of the note\n"));
    let (parsed, problems) = format::parse("x.md", &text);
    assert_eq!(problems, [] as [Problem; 0]);
    let activity = parsed.unwrap().activity;
    assert_eq!(activity.len(), 2);
    assert!(activity[1].text.contains("second line"));
}

#[test]
fn quotes_titles_that_would_break_yaml() {
    let mut task = Task::new(
        "FIX-1".into(),
        "Fix: a, b [c]".into(),
        Status::Todo,
        Priority::None,
        1000,
        Utc::now(),
    );
    task.body = "x".into();
    let text = format::render(&task);
    assert!(text.contains("title: \"Fix: a, b [c]\"\n"), "{text}");
    let (parsed, problems) = format::parse("x.md", &text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(parsed.unwrap().title, "Fix: a, b [c]");
}

// ─── strict parsing ─────────────────────────────────────────────────────────

fn problems_for(text: &str) -> Vec<Problem> {
    let (_, problems) = format::parse("FIX-1.md", text);
    problems
}

#[test]
fn rejects_a_bad_status_with_the_line_number() {
    let text = VALID_TASK.replace("status: todo", "status: inprogress");
    let problems = problems_for(&text);
    assert_ne!(problems, [] as [Problem; 0]);
    let problem = &problems[0];
    assert_eq!(problem.line, 4, "status is on line 4: {problem}");
    assert!(
        problem.message.contains("unknown status \"inprogress\""),
        "{problem}"
    );
    assert!(
        problem.message.contains("in_review"),
        "message lists the allowed set: {problem}"
    );
}

#[test]
fn rejects_a_missing_id() {
    let text = VALID_TASK.replace("id: FIX-900\n", "");
    let problems = problems_for(&text);
    assert!(
        problems.iter().any(|problem| problem
            .message
            .contains("missing required frontmatter key \"id\"")),
        "{problems:?}"
    );
}

#[test]
fn rejects_garbage_in_the_activity_section() {
    let text = format!("{VALID_TASK}this line is not an activity entry\n");
    let problems = problems_for(&text);
    let problem = problems
        .iter()
        .find(|problem| {
            problem
                .message
                .contains("unexpected content after `## Activity`")
        })
        .expect("garbage reported");
    assert_eq!(problem.line, 18);
}

#[test]
fn rejects_a_malformed_activity_entry() {
    let text = format!("{VALID_TASK}- not-a-date [user] hi\n");
    let problems = problems_for(&text);
    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("malformed activity entry")),
        "{problems:?}"
    );
}

#[test]
fn rejects_unknown_frontmatter_keys_and_values() {
    let text = VALID_TASK.replace("order: 1000", "order: one-thousand\nsprint: 4");
    let problems = problems_for(&text);
    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("order must be an integer")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|problem| problem
            .message
            .contains("unknown frontmatter key \"sprint\"")),
        "{problems:?}"
    );
}

#[test]
fn rejects_missing_frontmatter_and_unterminated_blocks() {
    let problems = problems_for("# not a task\n");
    assert!(problems[0].message.contains("missing frontmatter"));
    let problems = problems_for("---\nid: X-1\ntitle: t\n");
    assert!(problems[0].message.contains("unterminated frontmatter"));
}

#[test]
fn rejects_a_broken_run_block() {
    let text = VALID_TASK.replace("run:\n", "run:\n  session: s1\n  pid: twelve\n  host: h\n  mode: none\n  started: 2026-09-24T10:00:00Z\n");
    let problems = problems_for(&text);
    assert!(
        problems
            .iter()
            .any(|problem| problem.message.contains("run.pid must be a number")),
        "{problems:?}"
    );
}

// ─── validate --fix ─────────────────────────────────────────────────────────

#[test]
fn validate_reports_problems_and_fixes_only_formatting() {
    let fixture = Fixture::new();
    // A valid but non-canonical file (extra blank lines, missing trailing newline).
    let messy = VALID_TASK
        .replace("run:\n---", "run:\n\n---")
        .trim_end()
        .to_string();
    common::write_task_file(&fixture, "FIX-900", &messy);

    let result =
        commands::validate(&fixture.layout, fixture.project.clone(), false).expect("validate");
    assert_eq!(result.problems.len(), 1);
    assert!(result.problems[0].message.contains("formatting differs"));
    assert!(result.problems[0].fixable);

    let result =
        commands::validate(&fixture.layout, fixture.project.clone(), true).expect("validate --fix");
    assert!(result.problems.is_empty(), "{:?}", result.problems);
    assert_eq!(result.fixed, vec!["FIX-900".to_string()]);

    let after = commands::validate(&fixture.layout, fixture.project.clone(), false)
        .expect("validate again");
    assert!(after.problems.is_empty(), "{:?}", after.problems);
}

#[test]
fn validate_reports_board_level_rules_without_fixing_them() {
    let fixture = Fixture::new();
    // dangling dependency + cycle + in_progress without a run
    let with_dep = VALID_TASK
        .replace("id: FIX-900", "id: FIX-901")
        .replace("deps: []", "deps: [FIX-999]");
    common::write_task_file(&fixture, "FIX-901", &with_dep);
    let a = VALID_TASK
        .replace("id: FIX-900", "id: FIX-902")
        .replace("deps: []", "deps: [FIX-903]");
    let b = VALID_TASK
        .replace("id: FIX-900", "id: FIX-903")
        .replace("deps: []", "deps: [FIX-902]");
    common::write_task_file(&fixture, "FIX-902", &a);
    common::write_task_file(&fixture, "FIX-903", &b);
    let broken = VALID_TASK
        .replace("id: FIX-900", "id: FIX-904")
        .replace("status: todo", "status: in_progress");
    common::write_task_file(&fixture, "FIX-904", &broken);

    let result =
        commands::validate(&fixture.layout, fixture.project.clone(), true).expect("validate");
    let messages: Vec<&str> = result
        .problems
        .iter()
        .map(|problem| problem.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("dependency FIX-999 does not exist")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("dependency cycle")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("in_progress without a run block")),
        "{messages:?}"
    );
    // The dangling dep line number points at `deps:`.
    let dangling = result
        .problems
        .iter()
        .find(|p| p.message.contains("FIX-999"))
        .unwrap();
    assert_eq!(dangling.line, 7, "the `deps:` line: {dangling}");
    // Nothing was rewritten: the files are unchanged (no fixable problems here).
    assert_eq!(result.fixed, [] as [String; 0]);
}

#[test]
fn duplicate_ids_are_reported() {
    let fixture = Fixture::new();
    common::write_task_file(&fixture, "FIX-950", VALID_TASK);
    common::write_task_file(&fixture, "copy", VALID_TASK);
    let result =
        commands::validate(&fixture.layout, fixture.project.clone(), false).expect("validate");
    assert!(
        result
            .problems
            .iter()
            .any(|problem| problem.message.contains("duplicate task id FIX-900")),
        "{:?}",
        result.problems
    );
}

#[test]
fn activity_with_trailing_whitespace_stays_canonical() {
    let mut task = sample();
    // A note that ends with a newline (agents do this) used to render a blank
    // continuation line, which made the file permanently non-canonical and the
    // strict board refused to load it.
    task.push_activity(Utc::now(), Actor::Agent, "created the file\n");
    let text = format::render(&task);
    assert!(
        !text.contains("\n  \n"),
        "no blank continuation line: {text:?}"
    );
    let (parsed, problems) = format::parse("x.md", &text);
    assert!(problems.is_empty(), "{problems:?}");
    let parsed = parsed.unwrap();
    assert_eq!(parsed.activity.last().unwrap().text, "created the file");
    assert_eq!(format::render(&parsed), text, "render is idempotent");
}

#[test]
fn multi_line_notes_with_blank_lines_round_trip() {
    let mut task = sample();
    task.push_activity(Utc::now(), Actor::User, "line one\n\nline three\n\n");
    // Indented continuation lines are the case that used to break: the parser
    // trimmed them, so the file could never re-render canonically.
    task.push_activity(
        Utc::now(),
        Actor::User,
        "parent\n  indented child\n    deeper",
    );
    let text = format::render(&task);
    let (parsed, problems) = format::parse("x.md", &text);
    assert!(problems.is_empty(), "{problems:?}");
    let parsed = parsed.unwrap();
    assert_eq!(
        parsed.activity.last().unwrap().text,
        "parent\n  indented child\n    deeper"
    );
    // UNI-8: paragraph breaks survive (one empty line, never `  `).
    let paragraphs = &parsed.activity[parsed.activity.len() - 2].text;
    assert_eq!(paragraphs, "line one\n\nline three", "{text}");
    assert!(
        !text.contains("\n  \n"),
        "no whitespace-only line: {text:?}"
    );
    assert_eq!(format::render(&parsed), text);
    // And an indented note written through the CLI validates.
    let round_tripped = format::render(&parsed);
    let (again, problems) = format::parse("x.md", &round_tripped);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(format::render(&again.unwrap()), round_tripped);
}

#[test]
fn paragraph_breaks_collapse_and_never_join_the_next_entry() {
    let mut task = sample();
    task.push_activity(
        Utc::now(),
        Actor::Agent,
        "summary\n\n\n\n- a\n- b\n\n**Need:**\n1. x",
    );
    task.push_activity(Utc::now(), Actor::User, "next entry");
    let text = format::render(&task);
    let (parsed, problems) = format::parse("x.md", &text);
    assert!(problems.is_empty(), "{problems:?}");
    let parsed = parsed.unwrap();
    let n = parsed.activity.len();
    assert_eq!(
        parsed.activity[n - 2].text,
        "summary\n\n- a\n- b\n\n**Need:**\n1. x"
    );
    assert_eq!(parsed.activity[n - 1].text, "next entry");
    assert_eq!(format::render(&parsed), text, "render is idempotent");
}

#[test]
fn pi_settings_patch_validates_and_preserves_other_keys() {
    use oino_kanboard::serve::settings::{
        load_pi_settings_at, patch_pi_settings_at, validate_pi_patch,
    };
    // An explicit settings file in a temp dir; other top-level keys survive.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{"theme":"dark","extensionSettings":{"other":{"x":1}}}"#,
    )
    .unwrap();
    patch_pi_settings_at(
        &path,
        &serde_json::json!({"otherKey": "keep", "maxSessions": 3})
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    let whole: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(whole["theme"], "dark");
    assert_eq!(whole["extensionSettings"]["other"]["x"], 1);
    let stored = load_pi_settings_at(&path);
    assert_eq!(stored["maxSessions"], 3);
    assert_eq!(stored["otherKey"], "keep");
    // Valid patch validates.
    let good = validate_pi_patch(
        &serde_json::json!({"chainGate": "done", "retentionDays": 30})
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert_eq!(good["chainGate"], "done");
    // Unknown keys are refused.
    assert!(
        validate_pi_patch(
            &serde_json::json!({"queueMax": 5})
                .as_object()
                .unwrap()
                .clone()
        )
        .is_err()
    );
    // Invalid values are rejected.
    assert!(
        validate_pi_patch(
            &serde_json::json!({"chainGate": "bogus"})
                .as_object()
                .unwrap()
                .clone()
        )
        .is_err()
    );
    assert!(
        validate_pi_patch(
            &serde_json::json!({"maxSessions": 0})
                .as_object()
                .unwrap()
                .clone()
        )
        .is_err()
    );
    assert!(
        validate_pi_patch(&serde_json::json!({"nope": 1}).as_object().unwrap().clone()).is_err()
    );
}

/// UNI-58: quotes, backslashes, single quotes and newlines used to gain one
/// backslash per save — `unquote` re-escaped the escapes before parsing, so
/// `title: "say \"hi\""` parsed back as `say \"hi\"` and grew on every save.
/// Render→parse→render must be identity from the first pass, forever.
#[test]
fn hostile_title_roundtrips_repeatedly() {
    let titles = [
        r#"say "hi""#,
        r#"back\slash "and quotes""#,
        r"it's a 'plan', okay?",
        "line one\nline two",
        r#""quoted at both ends""#,
        r#"trailing "quote""#,
    ];
    for title in titles {
        let mut task = sample();
        task.title = title.to_string();
        let mut text = format::render(&task);
        for round in 0..4 {
            let (parsed, problems) = format::parse("t.md", &text);
            assert!(problems.is_empty(), "{title:?} round {round}: {problems:?}");
            let parsed = parsed.expect("usable frontmatter");
            assert_eq!(parsed.title, title, "round {round}");
            let next = format::render(&parsed);
            assert_eq!(next, text, "round {round}: rendering is not stable");
            text = next;
        }
    }
}

/// The run block's session/host go through the same unquote path.
#[test]
fn quoted_run_session_survives_a_roundtrip() {
    let mut task = sample();
    if let Some(run) = task.run.as_mut() {
        run.session = r#"weird "session" name"#.into();
    }
    let text = format::render(&task);
    let (parsed, problems) = format::parse("t.md", &text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        parsed.expect("usable").run.expect("run").session,
        r#"weird "session" name"#
    );
}
