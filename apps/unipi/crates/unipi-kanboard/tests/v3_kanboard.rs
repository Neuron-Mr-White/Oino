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
            max_sessions: 1,
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
    assert!(
        dispatcher::block(&f.layout, f.project.clone(), &t.id, "wrong", "question").unwrap()
            == false
    );
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
