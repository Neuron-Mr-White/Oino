//! Dispatcher mutations. The caller holds its machine-wide launch reservation; every
//! board read/check/write below additionally holds the existing project lock.
use crate::{
    Error, Result,
    board::Board,
    commands, deps,
    model::{Actor, Autonomy, ChainGate, Run, RunMode, RunOwner, Status, Task},
    store::{Layout, Project},
};
use chrono::Utc;
use serde_json::{Value, json};

/// Atomically reserve a ready ticket, enforcing project exclusion even against CLI claims.
pub fn reserve(layout: &Layout, project: Project, id: &str, session: &str) -> Result<Task> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let tasks = board.tasks()?;
    let mut task = board.get(id)?;
    if tasks.iter().any(|t| t.run.is_some()) {
        return Err(Error::rule(
            "Project already has a worker; finish or stop it first",
        ));
    }
    if task.status != Status::Todo {
        return Err(Error::rule(
            "Move the task to todo before starting a worker",
        ));
    }
    if deps::pending_deps_regardless_of_status(
        &task,
        &board.dep_lookup(&tasks),
        ChainGate::InReview,
    )
    .is_some()
    {
        return Err(Error::rule(
            "Finish the ticket dependencies before starting a worker",
        ));
    }
    task.status = Status::InProgress;
    task.run = Some(Run {
        session: session.into(),
        pid: std::process::id(),
        host: commands::hostname(),
        mode: RunMode::None,
        goal: None,
        started: Utc::now(),
        owner: RunOwner::Dispatcher,
    });
    task.push_activity_session(
        Utc::now(),
        Actor::System,
        None,
        format!("dispatcher reserved session {session}"),
    );
    board.save(&task)?;
    Ok(task)
}

/// Block only the still-matching dispatcher claim; never overwrite a worker's finish.
pub fn block(
    layout: &Layout,
    project: Project,
    id: &str,
    session: &str,
    question: &str,
) -> Result<bool> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let mut task = board.get(id)?;
    if !matches_claim(&task, session) {
        return Ok(false);
    }
    task.status = Status::Blocked;
    task.run = None;
    task.push_activity(Utc::now(), Actor::System, format!("blocked: {question}"));
    board.save(&task)?;
    Ok(true)
}

/// Park or resume an approval without releasing the dispatcher's durable claim.
pub fn approval_transition(
    layout: &Layout,
    project: Project,
    id: &str,
    session: &str,
    question: Option<&str>,
) -> Result<()> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let mut task = board.get(id)?;
    if !matches_claim(&task, session) {
        return Err(Error::rule(
            "Worker claim changed; cannot transition approval",
        ));
    }
    if let Some(question) = question {
        task.status = Status::Blocked;
        task.push_activity(Utc::now(), Actor::System, format!("blocked: {question}"));
    } else if task.status == Status::Blocked {
        task.status = Status::InProgress;
        task.push_activity(
            Utc::now(),
            Actor::System,
            "Approval answered; worker resumed",
        );
    }
    board.save(&task)
}

/// Validate and persist an explicit author-tagged relay under the claim lock.
/// Delivery must use the returned session, not a previously cached claimant.
pub fn relay(
    layout: &Layout,
    project: Project,
    id: &str,
    session: &str,
    text: &str,
    comment_id: &str,
) -> Result<Value> {
    if text.trim().is_empty() {
        return Err(Error::usage("Relay text must not be empty"));
    }
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let mut task = board.get(id)?;
    if !matches_claim(&task, session) {
        return Err(Error::rule(
            "Worker claim changed; refresh the board before relaying",
        ));
    }
    task.push_activity(
        Utc::now(),
        Actor::System,
        format!("relay {comment_id} [superagent]: {text}"),
    );
    board.save(&task)?;
    Ok(json!({"commentId": comment_id, "sessionId":session}))
}

/// Dispatcher ownership, status and session are all required for control operations.
#[must_use]
pub fn matches_claim(task: &Task, session: &str) -> bool {
    matches!(task.status, Status::InProgress | Status::Blocked)
        && task
            .run
            .as_ref()
            .is_some_and(|r| r.owner == RunOwner::Dispatcher && r.session == session)
}

/// Ticket ceilings always default to Medium, including legacy files.
#[must_use]
pub fn autonomy(task: &Task) -> Autonomy {
    task.autonomy.unwrap_or_default()
}
