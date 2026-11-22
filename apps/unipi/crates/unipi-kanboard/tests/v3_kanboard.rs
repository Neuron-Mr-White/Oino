//! v3 dispatcher claim, reaper, autonomy and relay contracts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;
use common::Fixture;
use unipi_kanboard::{
    commands::{self, StartArgs},
    dispatcher,
    model::{Autonomy, ChainGate, Priority, Status},
};

#[test]
fn batch_validation_does_not_partially_claim() {
    let f = Fixture::new();
    let a = f.add_with("ready", Status::Todo, Priority::Medium, &[]);
    let b = f.add_with(
        "waiting",
        Status::Todo,
        Priority::Medium,
        std::slice::from_ref(&a.id),
    );
    assert!(
        dispatcher::reserve_many(
            &f.layout,
            f.project.clone(),
            &[a.id.clone(), b.id.clone()],
            "lead"
        )
        .is_err()
    );
    let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
    assert_eq!(board.get(&a.id).unwrap().status, Status::Todo);
    assert!(board.get(&a.id).unwrap().run.is_none());
}

#[test]
fn dispatcher_session_reserves_multiple_idempotently_and_reaper_keeps_them() {
    let f = Fixture::new();
    let a = f.add_with("a", Status::Todo, Priority::Medium, &[]);
    let b = f.add_with("b", Status::Todo, Priority::Medium, &[]);
    dispatcher::reserve(&f.layout, f.project.clone(), &a.id, "lead").unwrap();
    dispatcher::reserve(&f.layout, f.project.clone(), &a.id, "lead").unwrap();
    dispatcher::reserve(&f.layout, f.project.clone(), &b.id, "lead").unwrap();
    assert!(dispatcher::reserve(&f.layout, f.project.clone(), &b.id, "other").is_err());
    let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
    for id in [&a.id, &b.id] {
        let mut task = board.get(id).unwrap();
        task.run.as_mut().unwrap().pid = u32::MAX;
        board.save(&task).unwrap();
    }
    commands::reap(&f.layout, f.project.clone(), false, f.common.now).unwrap();
    for id in [&a.id, &b.id] {
        assert_eq!(board.get(id).unwrap().run.unwrap().session, "lead");
    }
}

#[test]
fn dispatcher_reaper_does_not_requeue() {
    let f = Fixture::new();
    let t = f.add_with("dispatch", Status::Todo, Priority::Medium, &[]);
    dispatcher::reserve(&f.layout, f.project.clone(), &t.id, "reserved").unwrap();
    let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&t.id).unwrap();
    task.run.as_mut().unwrap().pid = u32::MAX;
    board.save(&task).unwrap();
    commands::reap(&f.layout, f.project.clone(), false, f.common.now).unwrap();
    assert_eq!(board.get(&t.id).unwrap().status, Status::InProgress);
}
#[test]
fn same_session_claim_is_idempotent() {
    let f = Fixture::new();
    let t = f.add_with("claim", Status::Todo, Priority::Medium, &[]);
    dispatcher::reserve(&f.layout, f.project.clone(), &t.id, "reserved").unwrap();
    let before = std::fs::read(f.layout.task_path(&f.project.slug, &t.id)).unwrap();
    commands::start(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &t.id,
        &StartArgs {
            session: "reserved",
            pid: std::process::id(),
            host: &commands::hostname(),
        },
        f.common.now,
    )
    .unwrap();
    assert_eq!(
        before,
        std::fs::read(f.layout.task_path(&f.project.slug, &t.id)).unwrap()
    );
}
#[test]
fn ticket_defaults_and_relay() {
    let f = Fixture::new();
    let t = f.add_with("legacy", Status::Todo, Priority::Medium, &[]);
    assert_eq!(dispatcher::autonomy(&t), Autonomy::Medium);
    dispatcher::reserve(&f.layout, f.project.clone(), &t.id, "right").unwrap();
    assert!(
        dispatcher::relay(
            &f.layout,
            f.project.clone(),
            &t.id,
            "wrong",
            "hello",
            "comment"
        )
        .is_err()
    );
    let reply = dispatcher::relay(
        &f.layout,
        f.project.clone(),
        &t.id,
        "right",
        "hello",
        "comment",
    )
    .unwrap();
    assert_eq!(reply["sessionId"], "right");
    let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&t.id).unwrap();
    task.autonomy = Some(Autonomy::High);
    board.save(&task).unwrap();
    assert_eq!(board.get(&t.id).unwrap().autonomy, Some(Autonomy::High));
    assert!(!dispatcher::block(&f.layout, f.project.clone(), &t.id, "wrong", "question").unwrap());
    assert!(dispatcher::block(&f.layout, f.project.clone(), &t.id, "right", "question").unwrap());
}

#[test]
fn autonomy_cli_create_edit_show_and_wrong_session_finish() {
    let f = Fixture::new();
    let create = common::cli_with_env(
        &f,
        &[
            "--json",
            "add",
            "ceiling",
            "--status",
            "todo",
            "--autonomy",
            "high",
        ],
        &[],
    );
    let created: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    assert_eq!(created["autonomy"], "high");
    let id = created["id"].as_str().unwrap();
    let edit = common::cli_with_env(&f, &["--json", "edit", id, "--autonomy", "low"], &[]);
    assert!(edit.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&edit.stdout).unwrap()["autonomy"],
        "low"
    );
    dispatcher::reserve(&f.layout, f.project.clone(), id, "right").unwrap();
    assert!(
        commands::finish(
            &f.layout,
            f.project.clone(),
            ChainGate::InReview,
            id,
            "wrong",
            "done",
            &[],
            f.common.now
        )
        .is_err()
    );
    assert!(
        commands::release(
            &f.layout,
            f.project.clone(),
            id,
            Status::Todo,
            "died",
            ChainGate::InReview,
            f.common.now
        )
        .is_err()
    );
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        id,
        "right",
        "done",
        &[],
        f.common.now,
    )
    .unwrap();
    assert_eq!(
        f.tasks().into_iter().find(|t| t.id == id).unwrap().status,
        Status::InReview
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep this linear integration scenario and its assertions together."
)]
fn unfinished_claim_releases_record_released_incomplete_activity() {
    use unipi_kanboard::model::{Actor, RunMode, RunOwner};

    let f = Fixture::new();

    // 1. `reap` (dead session PID) -> `session process gone`.
    let reaped_task = f.add_with("dead-pid", Status::Todo, Priority::Medium, &[]);
    f.start(&reaped_task.id, "sess-dead-full-id", 0x7fff_fffe);
    commands::reap(&f.layout, f.project.clone(), false, f.common.now).unwrap();
    let stored = f
        .tasks()
        .into_iter()
        .find(|t| t.id == reaped_task.id)
        .unwrap();
    assert_eq!(stored.status, Status::Todo);
    assert!(stored.run.is_none());
    let incomplete_lines: Vec<_> = stored
        .activity
        .iter()
        .filter(|a| a.text.starts_with(dispatcher::RELEASED_INCOMPLETE_PREFIX))
        .collect();
    assert_eq!(incomplete_lines.len(), 1);
    assert_eq!(incomplete_lines[0].actor, Actor::System);
    assert_eq!(
        incomplete_lines[0].text,
        "Released incomplete: was in_progress, held by session sess-dead-full-id (session process gone). Read the activity above and that session's history before continuing."
    );
    assert!(
        stored
            .handoff_notes()
            .iter()
            .any(|a| a.text.starts_with(dispatcher::RELEASED_INCOMPLETE_PREFIX))
    );

    // 2. `release` to `todo` -> `released by system`; `release` to `in_review` does NOT mark incomplete.
    let rel_todo = f.add_with("sys-release", Status::Todo, Priority::Medium, &[]);
    f.start(&rel_todo.id, "sess-rel-1", std::process::id());
    commands::release(
        &f.layout,
        f.project.clone(),
        &rel_todo.id,
        Status::Todo,
        "operator reset",
        ChainGate::InReview,
        f.common.now,
    )
    .unwrap();
    let stored = f.tasks().into_iter().find(|t| t.id == rel_todo.id).unwrap();
    assert_eq!(stored.status, Status::Todo);
    assert!(stored.run.is_none());
    assert_eq!(
        stored.activity.last().unwrap().text,
        "Released incomplete: was in_progress, held by session sess-rel-1 (released by system). Read the activity above and that session's history before continuing."
    );

    let rel_review = f.add_with("sys-review", Status::Todo, Priority::Medium, &[]);
    f.start(&rel_review.id, "sess-rel-2", std::process::id());
    commands::release(
        &f.layout,
        f.project.clone(),
        &rel_review.id,
        Status::InReview,
        "completed work",
        ChainGate::InReview,
        f.common.now,
    )
    .unwrap();
    let stored = f
        .tasks()
        .into_iter()
        .find(|t| t.id == rel_review.id)
        .unwrap();
    assert_eq!(stored.status, Status::InReview);
    assert!(
        !stored
            .activity
            .iter()
            .any(|a| a.text.starts_with(dispatcher::RELEASED_INCOMPLETE_PREFIX)),
        "completion to in_review is not incomplete"
    );

    // 3. Stale `move` by user (`in_progress -> backlog`) -> `released by user`.
    let stale_move = f.add_with("user-stale-release", Status::Todo, Priority::Medium, &[]);
    let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
    let mut manual = board.get(&stale_move.id).unwrap();
    manual.status = Status::InProgress;
    manual.run = Some(unipi_kanboard::model::Run {
        session: "sess-stale-user".into(),
        pid: 0x7fff_fffe,
        host: commands::hostname(),
        mode: RunMode::None,
        goal: None,
        started: f.common.now,
        owner: RunOwner::Agent,
    });
    board.save(&manual).unwrap();
    commands::move_task(
        &f.layout,
        f.project.clone(),
        &f.common,
        &stale_move.id,
        Status::Backlog,
        Some("stale process"),
    )
    .unwrap();
    let stored = f
        .tasks()
        .into_iter()
        .find(|t| t.id == stale_move.id)
        .unwrap();
    assert_eq!(stored.status, Status::Backlog);
    assert!(stored.run.is_none());
    assert_eq!(
        stored.activity.last().unwrap().text,
        "Released incomplete: was in_progress, held by session sess-stale-user (released by user). Read the activity above and that session's history before continuing."
    );

    // 4. `dispatcher::block` with `Worker ended without a transition` -> `worker ended without a transition`.
    let no_transition = f.add_with("worker-ended", Status::Todo, Priority::Medium, &[]);
    dispatcher::reserve(
        &f.layout,
        f.project.clone(),
        &no_transition.id,
        "sess-worker-end",
    )
    .unwrap();
    dispatcher::block(
        &f.layout,
        f.project.clone(),
        &no_transition.id,
        "sess-worker-end",
        "Worker ended without a transition; move to todo to retry",
    )
    .unwrap();
    let stored = f
        .tasks()
        .into_iter()
        .find(|t| t.id == no_transition.id)
        .unwrap();
    assert_eq!(stored.status, Status::Blocked);
    assert!(stored.run.is_none());
    assert_eq!(
        stored.activity.last().unwrap().text,
        "Released incomplete: was in_progress, held by session sess-worker-end (worker ended without a transition). Read the activity above and that session's history before continuing."
    );
}
