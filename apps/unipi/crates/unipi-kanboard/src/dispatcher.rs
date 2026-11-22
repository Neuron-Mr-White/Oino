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
    reserve_many(layout, project, &[id.to_owned()], session).map(|mut tasks| tasks.remove(0))
}

/// Reserve a batch under one board lock after validating every requested ticket.
pub fn reserve_many(
    layout: &Layout,
    project: Project,
    ids: &[String],
    session: &str,
) -> Result<Vec<Task>> {
    reserve_many_impl(layout, project, ids, session, false)
}

/// Reserve for an explicit manual start: backlog, todo and blocked tickets may be
/// taken directly. The watcher path keeps requiring a ready todo ticket.
pub fn reserve_many_manual(
    layout: &Layout,
    project: Project,
    ids: &[String],
    session: &str,
) -> Result<Vec<Task>> {
    reserve_many_impl(layout, project, ids, session, true)
}

fn reserve_many_impl(
    layout: &Layout,
    project: Project,
    ids: &[String],
    session: &str,
    manual: bool,
) -> Result<Vec<Task>> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let tasks = board.tasks()?;
    if ids.is_empty() {
        return Err(Error::usage("At least one ticket is required"));
    }
    if ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len() {
        return Err(Error::usage("Duplicate ticket id"));
    }
    let mut selected = Vec::with_capacity(ids.len());
    for id in ids {
        let task = board.get(id)?;
        if !matches_claim(&task, session) {
            // Per-ticket claims (OIN-82): a ticket is refused only when another
            // session already holds *that* ticket — the project may have other
            // sessions working other tickets at the same time.
            if let Some(run) = task.run.as_ref() {
                let sid8 = run.session.chars().take(8).collect::<String>();
                return Err(Error::rule(format!(
                    "{id} is held by {} session {sid8}. Pick another ticket, or stop that session first.",
                    run.owner.as_str()
                )));
            }
            if manual {
                if !matches!(
                    task.status,
                    Status::Backlog | Status::Todo | Status::Blocked
                ) {
                    return Err(Error::rule(format!(
                        "Cannot start a {} ticket; use backlog, todo or blocked",
                        task.status
                    )));
                }
            } else if task.status != Status::Todo {
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
        }
        selected.push(task);
    }
    for task in &mut selected {
        if matches_claim(task, session) && !(manual && task.status == Status::Blocked) {
            continue;
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
        board.save(task)?;
    }
    Ok(selected)
}

/// Prefix of the system activity entry written whenever an unfinished claim is released.
pub const RELEASED_INCOMPLETE_PREFIX: &str = "Released incomplete:";

/// Latest `blocked:` question text in a ticket's activity, if any.
#[must_use]
pub fn last_blocked_question(task: &Task) -> Option<&str> {
    task.activity
        .iter()
        .rev()
        .find_map(|entry| entry.text.strip_prefix("blocked:"))
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Format the system activity line marking an unfinished claim release.
#[must_use]
pub fn released_incomplete_line(
    was: Status,
    session: &str,
    reason: &str,
    open_question: Option<&str>,
) -> String {
    let extra = if was == Status::Blocked {
        open_question
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| format!("Open question: {q}. "))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "{RELEASED_INCOMPLETE_PREFIX} was {was}, held by session {session} ({reason}). {extra}Read the activity above and that session's history before continuing."
    )
}

fn block_incomplete_reason(question: &str) -> &'static str {
    let trimmed = question.trim();
    if trimmed.starts_with("Worker ended without a transition") {
        "worker ended without a transition"
    } else if trimmed.starts_with("Worker was interrupted") {
        "worker was interrupted"
    } else if trimmed.starts_with("Worker stopped") {
        "worker stopped"
    } else if trimmed.starts_with("Lead delivery failed") {
        "lead delivery failed"
    } else if trimmed.starts_with("Worker could not start") {
        "worker could not start"
    } else if trimmed.starts_with("Worker ended while blocked") {
        "worker ended while blocked"
    } else {
        "dispatcher released claim"
    }
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
    let was = task.status;
    let prev_question = (was == Status::Blocked)
        .then(|| last_blocked_question(&task).map(str::to_owned))
        .flatten();
    task.status = Status::Blocked;
    task.run = None;
    let now = Utc::now();
    task.push_activity(now, Actor::System, format!("blocked: {question}"));
    task.push_activity(
        now,
        Actor::System,
        released_incomplete_line(
            was,
            session,
            block_incomplete_reason(question),
            prev_question.as_deref(),
        ),
    );
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

/// The owner answered a lead's blocked question: return the ticket to
/// `in_progress` under the same dispatcher claim and record the answer on it
/// (OIN-81). Err when the ticket is not blocked (already answered or moved).
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniform dispatcher signature: every function takes the Project by value"
)]
pub fn resume_blocked(
    layout: &Layout,
    project: Project,
    id: &str,
    session: &str,
    answer: &str,
) -> Result<Task> {
    let _lock = layout.lock_board(&project.slug)?;
    let board = Board::open(layout, project)?;
    let mut task = board.get(id)?;
    if task.status != Status::Blocked {
        return Err(Error::rule(format!(
            "{id} is {}, not blocked — the question was already answered or the ticket moved on",
            task.status
        )));
    }
    if !matches_claim(&task, session) {
        return Err(Error::rule(
            "Worker claim changed; cannot resume blocked ticket",
        ));
    }
    task.status = Status::InProgress;
    task.push_activity(Utc::now(), Actor::User, format!("Owner answer: {answer}"));
    board.save(&task)?;
    Ok(task)
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
#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _home: tempfile::TempDir,
        _root: tempfile::TempDir,
        layout: Layout,
        project: Project,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("home");
            let root = tempfile::tempdir().expect("root");
            let layout = Layout::with_home(home.path());
            let project = Project::create(&layout, root.path(), Some("Fixture"), Some("FIX"))
                .expect("project");
            Fixture {
                _home: home,
                _root: root,
                layout,
                project,
            }
        }

        fn todo_task(&self, title: &str) -> String {
            let common = commands::Common::new(Actor::User, ChainGate::InReview);
            let value = commands::add(
                &self.layout,
                self.project.clone(),
                &common,
                title,
                Some("body"),
                Some(Status::Todo),
                crate::model::Priority::None,
                &[],
                &[],
                &[],
                false,
            )
            .expect("add");
            value["id"].as_str().expect("id").to_string()
        }

        fn stored(&self, id: &str) -> Task {
            Board::open(&self.layout, self.project.clone())
                .expect("board")
                .get(id)
                .expect("task")
        }
    }

    /// OIN-81: the owner's answer resumes a blocked question in place — same
    /// claim, one activity line, and a second answer is refused.
    #[test]
    fn resume_blocked_reopens_the_ticket_once_and_records_the_answer() {
        let f = Fixture::new();
        let id = f.todo_task("question");
        reserve(&f.layout, f.project.clone(), &id, "lead").expect("reserve");
        approval_transition(
            &f.layout,
            f.project.clone(),
            &id,
            "lead",
            Some("which log format?"),
        )
        .expect("park with claim");
        assert_eq!(f.stored(&id).status, Status::Blocked);
        assert_eq!(
            f.stored(&id).run.as_ref().map(|r| r.session.as_str()),
            Some("lead")
        );

        let task = resume_blocked(
            &f.layout,
            f.project.clone(),
            &id,
            "lead",
            "plain logs please",
        )
        .expect("resume");
        assert_eq!(task.status, Status::InProgress);
        let stored = f.stored(&id);
        assert_eq!(stored.status, Status::InProgress);
        assert_eq!(
            stored.run.as_ref().map(|r| r.session.as_str()),
            Some("lead")
        );
        let last = stored.activity.last().expect("activity");
        assert_eq!(last.actor, Actor::User);
        assert_eq!(last.text, "Owner answer: plain logs please");

        // Not blocked any more: a second answer is refused (R1 double answer).
        let err = resume_blocked(&f.layout, f.project.clone(), &id, "lead", "again").unwrap_err();
        assert!(err.to_string().contains("not blocked"), "{err}");
    }

    #[test]
    fn resume_blocked_rechecks_claimant_under_board_lock() {
        let f = Fixture::new();
        let id = f.todo_task("claimed-question");
        reserve(&f.layout, f.project.clone(), &id, "lead-a").expect("reserve");
        approval_transition(
            &f.layout,
            f.project.clone(),
            &id,
            "lead-a",
            Some("which database?"),
        )
        .expect("park with claim");

        // Wrong session id is refused without mutating status or activity.
        let before_len = f.stored(&id).activity.len();
        let err =
            resume_blocked(&f.layout, f.project.clone(), &id, "lead-b", "postgres").unwrap_err();
        assert!(err.to_string().contains("claim changed"), "{err}");
        assert_eq!(f.stored(&id).status, Status::Blocked);
        assert_eq!(f.stored(&id).activity.len(), before_len);
        assert_eq!(
            f.stored(&id).run.as_ref().map(|r| r.session.as_str()),
            Some("lead-a")
        );

        // A blocked ticket whose dispatcher claim was released (`run = None`) is also refused.
        let released = f.todo_task("released-question");
        reserve(&f.layout, f.project.clone(), &released, "lead-a").expect("reserve");
        block(&f.layout, f.project.clone(), &released, "lead-a", "stopped").expect("block");
        assert!(f.stored(&released).run.is_none());
        let err = resume_blocked(
            &f.layout,
            f.project.clone(),
            &released,
            "lead-a",
            "postgres",
        )
        .unwrap_err();
        assert!(err.to_string().contains("claim changed"), "{err}");
        assert_eq!(f.stored(&released).status, Status::Blocked);
    }
}
