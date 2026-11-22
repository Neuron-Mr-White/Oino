//! Every row of the transition table: allowed actors, denials, comment rules,
//! and final states.
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

use common::Fixture;
use unipi_kanboard::commands;
use unipi_kanboard::dispatcher;
use unipi_kanboard::error::Error;
use unipi_kanboard::model::{Actor, Priority, Staleness, Status};
use unipi_kanboard::transitions::{self, RULES, comment_required, rule_for};

fn allowed(
    from: Status,
    to: Status,
    actor: Actor,
    comment: Option<&str>,
    staleness: Staleness,
) -> bool {
    transitions::check(from, to, actor, comment, staleness).is_ok()
}

const REASON: Option<&str> = Some("because");

#[test]
fn every_table_row_is_reachable_and_gated() {
    assert!(!RULES.is_empty());
    for rule in RULES {
        // The comment rule is enforced where the table demands it (an agent
        // cancel always needs one, OIN-89).
        let actor = rule.actors[0];
        let needs_comment = comment_required(rule.from, rule.to)
            || (rule.to == Status::Cancelled && actor == Actor::Agent);
        let comment = if needs_comment { REASON } else { None };
        assert!(
            allowed(rule.from, rule.to, actor, comment, Staleness::Stale),
            "{} → {} should be allowed for {}",
            rule.from,
            rule.to,
            actor
        );

        // And a missing comment is refused when the row demands one.
        if needs_comment {
            let err =
                transitions::check(rule.from, rule.to, actor, None, Staleness::Stale).unwrap_err();
            let message = err.to_string();
            assert!(
                message.contains("requires --comment"),
                "unexpected message: {message}"
            );
            assert!(message.contains(rule.from.as_str()) && message.contains(rule.to.as_str()));
        }
    }
}

/// UNI-106: a rework note is no longer mandatory — `in_review` → todo goes
/// through with no comment at all, and still works when one is given.
#[test]
fn rework_note_is_optional() {
    transitions::check(
        Status::InReview,
        Status::Todo,
        Actor::User,
        None,
        Staleness::Running,
    )
    .expect("in_review → todo needs no comment");
    transitions::check(
        Status::InReview,
        Status::Todo,
        Actor::User,
        Some("needs tests"),
        Staleness::Running,
    )
    .expect("in_review → todo still accepts a comment");
}

#[test]
fn blocked_requires_a_comment_from_agent_and_system() {
    for actor in [Actor::Agent, Actor::System] {
        let err = transitions::check(
            Status::InProgress,
            Status::Blocked,
            actor,
            None,
            Staleness::Running,
        )
        .unwrap_err();
        assert!(err.to_string().contains("requires --comment"), "{err}");
        assert!(allowed(
            Status::InProgress,
            Status::Blocked,
            actor,
            REASON,
            Staleness::Running
        ));
    }
    // The user cannot block a running task (the agent owns that move).
    assert!(!allowed(
        Status::InProgress,
        Status::Blocked,
        Actor::User,
        REASON,
        Staleness::Running
    ));
}

/// UNI-105: an agent may unblock a task to todo (e.g. to resume it with
/// `start`); UNI-106: the answer is optional for both actors.
#[test]
fn unblocking_is_free_of_a_comment_and_open_to_agents() {
    transitions::check(
        Status::Blocked,
        Status::Todo,
        Actor::User,
        None,
        Staleness::Running,
    )
    .expect("blocked → todo needs no comment");
    assert!(allowed(
        Status::Blocked,
        Status::Todo,
        Actor::User,
        Some("use json logs"),
        Staleness::Running
    ));
    // UNI-105: agents may unblock too, with or without a comment.
    assert!(allowed(
        Status::Blocked,
        Status::Todo,
        Actor::Agent,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Blocked,
        Status::Todo,
        Actor::Agent,
        REASON,
        Staleness::Running
    ));
}

#[test]
fn start_and_finish_are_agent_and_system_only() {
    assert!(!allowed(
        Status::Todo,
        Status::InProgress,
        Actor::User,
        None,
        Staleness::Running
    ));
    // The agent self-claims with `start` (the table row; `start` enforces the
    // claim rules, `move` refuses it).
    assert!(allowed(
        Status::Todo,
        Status::InProgress,
        Actor::Agent,
        None,
        Staleness::Running
    ));
    // …and hands its own claim to review with `finish` (summary required).
    assert!(allowed(
        Status::InProgress,
        Status::InReview,
        Actor::Agent,
        REASON,
        Staleness::Running
    ));
    assert!(!allowed(
        Status::InProgress,
        Status::InReview,
        Actor::Agent,
        None,
        Staleness::Running
    ));
    let err = transitions::check(
        Status::Todo,
        Status::InProgress,
        Actor::User,
        None,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("Only an agent session can claim"),
        "{err}"
    );
    assert!(allowed(
        Status::Todo,
        Status::InProgress,
        Actor::System,
        None,
        Staleness::Running
    ));

    let err = transitions::check(
        Status::InProgress,
        Status::InReview,
        Actor::User,
        REASON,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Only the session that started it hands it over"),
        "{err}"
    );
    assert!(allowed(
        Status::InProgress,
        Status::InReview,
        Actor::System,
        REASON,
        Staleness::Running
    ));
}

#[test]
fn a_running_task_cannot_be_moved_by_a_user() {
    let err = transitions::check(
        Status::InProgress,
        Status::Todo,
        Actor::User,
        REASON,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("while its session is alive is agent/system only"),
        "{err}"
    );
    let err = transitions::check(
        Status::InProgress,
        Status::Todo,
        Actor::User,
        REASON,
        Staleness::Unknown,
    )
    .unwrap_err();
    assert!(err.to_string().contains("another host"), "{err}");
    // A confirmed stale run may be released by the user.
    assert!(allowed(
        Status::InProgress,
        Status::Todo,
        Actor::User,
        REASON,
        Staleness::Stale
    ));
    assert!(allowed(
        Status::InProgress,
        Status::Backlog,
        Actor::User,
        REASON,
        Staleness::Stale
    ));
    // …but always with a note.
    assert!(!allowed(
        Status::InProgress,
        Status::Todo,
        Actor::User,
        None,
        Staleness::Stale
    ));
}

#[test]
fn agents_cannot_finish_but_may_cancel_with_a_reason() {
    // Done stays owner-only.
    for from in [
        Status::InReview,
        Status::Todo,
        Status::Backlog,
        Status::Blocked,
        Status::InProgress,
    ] {
        assert!(
            !allowed(from, Status::Done, Actor::Agent, REASON, Staleness::Running),
            "agent must not move {from} → done"
        );
    }
    let err = transitions::check(
        Status::InReview,
        Status::Done,
        Actor::Agent,
        None,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(err.to_string().contains("can't mark tickets done"), "{err}");

    // OIN-89: an agent may abandon a ticket it could work, but only with a
    // reason; in_review stays forbidden.
    for from in [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::Blocked,
    ] {
        assert!(
            allowed(
                from,
                Status::Cancelled,
                Actor::Agent,
                REASON,
                Staleness::Running
            ),
            "agent should cancel {from} → cancelled"
        );
        let err = transitions::check(
            from,
            Status::Cancelled,
            Actor::Agent,
            None,
            Staleness::Running,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("requires --comment"),
            "agent cancel from {from} must require a reason: {err}"
        );
    }
    assert!(!allowed(
        Status::InReview,
        Status::Cancelled,
        Actor::Agent,
        REASON,
        Staleness::Running
    ));
}

#[test]
fn backlog_and_todo_shift_freely_for_user_and_agent() {
    assert!(allowed(
        Status::Backlog,
        Status::Todo,
        Actor::User,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Todo,
        Status::Backlog,
        Actor::User,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Backlog,
        Status::Todo,
        Actor::Agent,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Todo,
        Status::Backlog,
        Actor::Agent,
        None,
        Staleness::Running
    ));
}

#[test]
fn final_states_are_final() {
    for final_status in [Status::Done, Status::Cancelled, Status::Archived] {
        assert!(final_status.is_final());
        for to in Status::ALL {
            // Archiving is the one documented exit for done/cancelled.
            if to == final_status || to == Status::Archived {
                continue;
            }
            assert!(
                !allowed(final_status, to, Actor::User, REASON, Staleness::Stale),
                "{final_status} → {to} must be refused"
            );
        }
    }
    let err = transitions::check(
        Status::Done,
        Status::Todo,
        Actor::User,
        REASON,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(err.to_string().contains("final"), "{err}");
    assert!(
        err.to_string().contains("duplicate"),
        "hint at duplicate: {err}"
    );
    let err = transitions::check(
        Status::Archived,
        Status::Todo,
        Actor::User,
        REASON,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(err.to_string().contains("archived is final"), "{err}");
}

#[test]
fn user_cancel_is_allowed_from_the_open_lanes() {
    for from in [Status::Backlog, Status::Todo, Status::Blocked] {
        assert!(allowed(
            from,
            Status::Cancelled,
            Actor::User,
            None,
            Staleness::Running
        ));
    }
    assert!(!allowed(
        Status::InReview,
        Status::Cancelled,
        Actor::User,
        None,
        Staleness::Running
    ));
    let err = transitions::check(
        Status::InReview,
        Status::Cancelled,
        Actor::User,
        None,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("rework note"),
        "hint the path back: {err}"
    );
}

#[test]
fn archiving_is_the_only_exit_from_done_and_cancelled() {
    assert!(allowed(
        Status::Done,
        Status::Archived,
        Actor::User,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Cancelled,
        Status::Archived,
        Actor::User,
        None,
        Staleness::Running
    ));
    assert!(allowed(
        Status::Done,
        Status::Archived,
        Actor::System,
        None,
        Staleness::Running
    ));
    assert!(!allowed(
        Status::Todo,
        Status::Archived,
        Actor::User,
        None,
        Staleness::Running
    ));
}

#[test]
fn same_status_is_refused() {
    let err = transitions::check(
        Status::Todo,
        Status::Todo,
        Actor::User,
        None,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(err.to_string().contains("already in todo"), "{err}");
}

#[test]
fn allowed_targets_are_listed_per_actor() {
    let user_targets = transitions::allowed_targets(Status::InReview, Actor::User);
    assert!(user_targets.contains(&Status::Done));
    assert!(user_targets.contains(&Status::Todo));
    assert!(!user_targets.contains(&Status::InProgress));
    let agent_targets = transitions::allowed_targets(Status::InReview, Actor::Agent);
    assert!(agent_targets.is_empty(), "{agent_targets:?}");
}

#[test]
fn the_table_has_no_duplicate_rows() {
    for (index, rule) in RULES.iter().enumerate() {
        for other in RULES.iter().skip(index + 1) {
            assert!(
                !(rule.from == other.from && rule.to == other.to),
                "duplicate row {} → {}",
                rule.from,
                rule.to
            );
        }
    }
}

// ─── end-to-end through the board (real files, real locks) ──────────────────

#[test]
fn agent_blocks_declares_a_comment_and_the_row_becomes_readable() {
    let fixture = Fixture::new();
    let task = fixture.add_with("agent task", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());

    let mut agent_common = fixture.common.clone();
    agent_common.actor = Actor::Agent;
    agent_common.session = Some("sess-1".to_string());

    let err = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Blocked,
        None,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Rule(_)));
    assert!(err.to_string().contains("requires --comment"), "{err}");

    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Blocked,
        Some("which log format?"),
    )
    .expect("block");
    assert_eq!(value["status"], "blocked");
    let activity = value["activity"].as_array().unwrap();
    assert!(activity
        .iter()
        .any(|entry| entry["text"] == "blocked: which log format?" && entry["actor"] == "agent"));
}

#[test]
fn the_user_answers_a_blocked_task_and_the_note_survives() {
    let fixture = Fixture::new();
    let task = fixture.add_with("blocked task", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let mut agent_common = fixture.common.clone();
    agent_common.actor = Actor::Agent;
    agent_common.session = Some("sess-1".to_string());
    commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Blocked,
        Some("need the log format"),
    )
    .expect("block");

    // UNI-106: the answer is optional now — unblock with a comment still
    // carries it through to the activity text.
    commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &fixture.common,
        &task.id,
        Status::Todo,
        Some("json logs"),
    )
    .expect("unblock");

    let stored = fixture
        .tasks()
        .into_iter()
        .find(|candidate| candidate.id == task.id)
        .unwrap();
    assert_eq!(stored.status, Status::Todo);
    assert!(stored.run.is_none(), "unblocking clears the run block");
    assert!(
        stored
            .activity
            .iter()
            .any(|entry| entry.text == "unblocked: json logs")
    );
    assert!(
        stored
            .handoff_notes()
            .iter()
            .any(|entry| entry.text.contains("json logs"))
    );
}

/// UNI-105 / UNI-106: an agent may unblock its own blocked task with no
/// comment at all — the bare activity text carries no trailing colon.
#[test]
fn an_agent_unblocks_without_a_comment_and_the_text_carries_no_colon() {
    let fixture = Fixture::new();
    let task = fixture.add_with("blocked by agent", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let mut agent_common = fixture.common.clone();
    agent_common.actor = Actor::Agent;
    agent_common.session = Some("sess-1".to_string());
    commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Blocked,
        Some("need the log format"),
    )
    .expect("block");

    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Todo,
        None,
    )
    .expect("agent unblocks itself without a comment");
    assert_eq!(value["status"], "todo");
    let activity = value["activity"].as_array().unwrap();
    assert_eq!(activity.last().unwrap()["text"], "unblocked");
}

/// OIN-89: an agent may cancel a backlog/todo ticket (wrongly-filed or
/// obsolete cleanup), but must say why.
#[test]
fn agent_cancel_from_backlog_and_todo_requires_a_reason() {
    let fixture = Fixture::new();
    let mut agent = fixture.common.clone();
    agent.actor = Actor::Agent;
    agent.session = Some("sess-clean".to_string());

    for (title, status) in [
        ("filed on the wrong board", Status::Backlog),
        ("obsolete", Status::Todo),
    ] {
        let task = fixture.add_with(title, status, Priority::None, &[]);
        let err = commands::move_task(
            &fixture.layout,
            fixture.project.clone(),
            &agent,
            &task.id,
            Status::Cancelled,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("requires --comment"), "{err}");

        let value = commands::move_task(
            &fixture.layout,
            fixture.project.clone(),
            &agent,
            &task.id,
            Status::Cancelled,
            Some("filed on the wrong board"),
        )
        .expect("agent cancels its own backlog/todo ticket");
        assert_eq!(value["status"], "cancelled");

        let stored = fixture
            .tasks()
            .into_iter()
            .find(|candidate| candidate.id == task.id)
            .unwrap();
        assert_eq!(stored.status, Status::Cancelled);
        assert!(stored.run.is_none());
        assert!(
            stored.activity.iter().any(|entry| {
                entry.text == "cancelled: filed on the wrong board" && entry.actor == Actor::Agent
            }),
            "{:?}",
            stored.activity
        );
    }
}

/// OIN-89: an agent may cancel an in_progress ticket it holds, and only that
/// session; the cancel clears the claim.
#[test]
fn agent_cancel_of_in_progress_needs_the_claiming_session_and_clears_the_run() {
    let fixture = Fixture::new();
    let task = fixture.add_with("working", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let mut owner = fixture.common.clone();
    owner.actor = Actor::Agent;
    owner.session = Some("sess-1".to_string());

    let mut other = owner.clone();
    other.session = Some("sess-2".to_string());
    let err = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &other,
        &task.id,
        Status::Cancelled,
        Some("abandon"),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("only while its own session holds it"),
        "wrong session: {err}"
    );

    let err = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &owner,
        &task.id,
        Status::Cancelled,
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("requires --comment"), "{err}");

    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &owner,
        &task.id,
        Status::Cancelled,
        Some("no longer needed"),
    )
    .expect("the holder cancels");
    assert_eq!(value["status"], "cancelled");
    let stored = fixture
        .tasks()
        .into_iter()
        .find(|candidate| candidate.id == task.id)
        .unwrap();
    assert!(stored.run.is_none(), "cancelling clears the run block");
}

/// OIN-89: an agent-blocked ticket has already released its claim, so a
/// worker can abandon it (with a reason) instead of getting stuck.
#[test]
fn an_agent_may_cancel_a_blocked_ticket_with_no_claim() {
    let fixture = Fixture::new();
    let task = fixture.add_with("blocked", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let mut owner = fixture.common.clone();
    owner.actor = Actor::Agent;
    owner.session = Some("sess-1".to_string());
    commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &owner,
        &task.id,
        Status::Blocked,
        Some("need the key"),
    )
    .expect("block");

    let mut other = owner.clone();
    other.session = Some("sess-2".to_string());
    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &other,
        &task.id,
        Status::Cancelled,
        Some("the question is moot"),
    )
    .expect("an unclaimed blocked ticket can be abandoned");
    assert_eq!(value["status"], "cancelled");
}

/// OIN-89: a dispatcher (lead) ticket blocked while an approval is parked can
/// still be abandoned by the claiming session.
#[test]
fn an_agent_may_cancel_the_dispatcher_blocked_ticket_it_holds() {
    let fixture = Fixture::new();
    let task = fixture.add_with("approval", Status::Todo, Priority::None, &[]);
    dispatcher::reserve(&fixture.layout, fixture.project.clone(), &task.id, "lead-1")
        .expect("reserve");
    dispatcher::approval_transition(
        &fixture.layout,
        fixture.project.clone(),
        &task.id,
        "lead-1",
        Some("which key?"),
    )
    .expect("park the approval while keeping the claim");

    let mut other = fixture.common.clone();
    other.actor = Actor::Agent;
    other.session = Some("sess-2".to_string());
    let err = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &other,
        &task.id,
        Status::Cancelled,
        Some("abandon"),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("only while its own session holds it"),
        "{err}"
    );

    let mut lead = fixture.common.clone();
    lead.actor = Actor::Agent;
    lead.session = Some("lead-1".to_string());
    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &lead,
        &task.id,
        Status::Cancelled,
        Some("approval no longer needed"),
    )
    .expect("the lead abandons its blocked ticket");
    assert_eq!(value["status"], "cancelled");
    let stored = fixture
        .tasks()
        .into_iter()
        .find(|candidate| candidate.id == task.id)
        .unwrap();
    assert!(stored.run.is_none());
}

#[test]
fn duplicate_resets_lifecycle_fields() {
    let fixture = Fixture::new();
    let task = fixture.add_with("original", Status::Todo, Priority::High, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let value = commands::duplicate(
        &fixture.layout,
        fixture.project.clone(),
        &fixture.common,
        &task.id,
    )
    .unwrap();
    assert_eq!(value["status"], "backlog");
    assert_eq!(value["priority"], "high");
    assert!(value["run"].is_null());
    assert_ne!(value["id"], task.id.as_str());
}

#[test]
fn rule_for_unknown_pairs_is_none() {
    assert!(rule_for(Status::Backlog, Status::Done).is_none());
    assert!(!comment_required(Status::Backlog, Status::Todo));
}

/// OIN-28: parking a blocked task in the backlog logs
/// `moved blocked → backlog: …` and leaves no claim behind.
#[test]
fn blocked_to_backlog_lands_in_backlog_with_activity_and_no_claim() {
    let fixture = Fixture::new();
    let task = fixture.add_with("blocked task", Status::Todo, Priority::None, &[]);
    fixture.start(&task.id, "sess-1", std::process::id());
    let mut agent_common = fixture.common.clone();
    agent_common.actor = Actor::Agent;
    agent_common.session = Some("sess-1".to_string());
    commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &agent_common,
        &task.id,
        Status::Blocked,
        Some("need the log format"),
    )
    .expect("block");

    let value = commands::move_task(
        &fixture.layout,
        fixture.project.clone(),
        &fixture.common,
        &task.id,
        Status::Backlog,
        Some("park until Q1"),
    )
    .expect("user parks blocked → backlog");
    assert_eq!(value["status"], "backlog");
    assert!(
        value["allowedMoves"]
            .as_array()
            .unwrap()
            .iter()
            .any(|move_| move_.as_str() == Some("todo"))
    );

    let stored = fixture
        .tasks()
        .into_iter()
        .find(|candidate| candidate.id == task.id)
        .unwrap();
    assert_eq!(stored.status, Status::Backlog);
    assert!(
        stored.run.is_none(),
        "parking in the backlog clears the claim"
    );
    assert!(
        stored
            .activity
            .iter()
            .any(|entry| entry.text == "moved blocked → backlog: park until Q1")
    );
}

/// UNI-63: a user may close a blocked task directly; agents still cannot mark
/// anything done.
#[test]
fn blocked_to_done_is_user_only() {
    transitions::check(
        Status::Blocked,
        Status::Done,
        Actor::User,
        None,
        Staleness::Running,
    )
    .expect("user blocked → done needs no comment");
    let err = transitions::check(
        Status::Blocked,
        Status::Done,
        Actor::Agent,
        None,
        Staleness::Running,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("Agents can't mark tickets done"),
        "{err}"
    );
    assert_eq!(
        transitions::allowed_targets(Status::Blocked, Actor::User),
        vec![
            Status::Todo,
            Status::Backlog,
            Status::Done,
            Status::Cancelled
        ]
    );
}

/// OIN-28: a blocked task may be parked back in the backlog by user, agent, or
/// system (e.g. retiring a lead's blocked claim).
#[test]
fn blocked_to_backlog_is_user_and_agent_only() {
    transitions::check(
        Status::Blocked,
        Status::Backlog,
        Actor::User,
        None,
        Staleness::Running,
    )
    .expect("user blocked → backlog needs no comment");
    transitions::check(
        Status::Blocked,
        Status::Backlog,
        Actor::Agent,
        None,
        Staleness::Running,
    )
    .expect("agent blocked → backlog needs no comment");
    transitions::check(
        Status::Blocked,
        Status::Backlog,
        Actor::System,
        None,
        Staleness::Running,
    )
    .expect("system blocked → backlog needs no comment");

    let targets = transitions::allowed_targets(Status::Blocked, Actor::User);
    assert!(targets.contains(&Status::Todo), "{targets:?}");
    assert!(targets.contains(&Status::Backlog), "{targets:?}");
}

/// UNI-57: which completed moves carry an undo offer — terminal lanes whose
/// reverse is not a plain user move; routine reversible moves never do.
#[test]
fn undo_worthy_matches_the_hard_to_reverse_moves() {
    assert!(transitions::undo_worthy(
        Status::InReview,
        Status::Done,
        Actor::User
    ));
    assert!(transitions::undo_worthy(
        Status::Todo,
        Status::Cancelled,
        Actor::User
    ));
    assert!(transitions::undo_worthy(
        Status::Backlog,
        Status::Cancelled,
        Actor::User
    ));
    assert!(transitions::undo_worthy(
        Status::Blocked,
        Status::Done,
        Actor::User
    ));
    assert!(transitions::undo_worthy(
        Status::Done,
        Status::Archived,
        Actor::User
    ));
    assert!(transitions::undo_worthy(
        Status::InReview,
        Status::Archived,
        Actor::User
    ));
    // Routine, normally reversible moves are not undo-worthy.
    assert!(!transitions::undo_worthy(
        Status::Backlog,
        Status::Todo,
        Actor::User
    ));
    assert!(!transitions::undo_worthy(
        Status::Todo,
        Status::Backlog,
        Actor::User
    ));
    assert!(!transitions::undo_worthy(
        Status::Blocked,
        Status::Todo,
        Actor::User
    ));
    // Never for agents, never out of a live claim.
    assert!(!transitions::undo_worthy(
        Status::Todo,
        Status::Cancelled,
        Actor::Agent
    ));
    assert!(!transitions::undo_worthy(
        Status::InProgress,
        Status::Blocked,
        Actor::User
    ));
}

// ─── OIN-82: denial texts name the next step, and no "honour system" ────────

/// Every agent denial says what to do instead (the command that works), and
/// none of them mentions the honour system.
#[test]
fn denied_texts_name_next_step_and_no_honour_system() {
    let denied = |from: Status, to: Status, actor: Actor| {
        transitions::check(from, to, actor, None, Staleness::Running)
            .unwrap_err()
            .to_string()
    };

    let cases: [(Status, Status, Actor, &str); 5] = [
        (
            Status::Todo,
            Status::InProgress,
            Actor::User,
            "hand it to a lead with Start in the work pane",
        ),
        (
            Status::Blocked,
            Status::InProgress,
            Actor::User,
            "answer the question in the work pane",
        ),
        (
            Status::InProgress,
            Status::InReview,
            Actor::User,
            "finish <ID> --comment",
        ),
        (
            Status::InReview,
            Status::Done,
            Actor::Agent,
            "the owner marks it done",
        ),
        (
            Status::InReview,
            Status::Cancelled,
            Actor::Agent,
            "rework note",
        ),
    ];
    for (from, to, actor, next_step) in cases {
        let text = denied(from, to, actor);
        assert!(text.contains(next_step), "{from} → {to} as {actor}: {text}");
        assert!(
            !text.contains("honour system"),
            "{from} → {to} as {actor}: {text}"
        );
    }

    // A pair whose rule exists but has no dedicated text falls back to the
    // generic arm: the message names the actors and the lanes the actor
    // *could* use instead (in_review → todo is user-only).
    let text = denied(Status::InReview, Status::Todo, Actor::Agent);
    assert!(text.contains("is for user only"), "{text}");
    assert!(
        text.contains("From in_review you can move to: done, todo, backlog"),
        "{text}"
    );
    assert!(!text.contains("honour system"), "{text}");
}
