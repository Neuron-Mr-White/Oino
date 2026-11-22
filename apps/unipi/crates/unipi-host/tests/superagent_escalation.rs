#![allow(clippy::unwrap_used, clippy::expect_used)]
use futures_util::future::BoxFuture;
use std::sync::{Arc, Mutex};
use unipi_host::superagent::{Dispatcher, Workers};
use unipi_kanboard::{
    board::Board,
    commands::{self, Common},
    model::{Actor, ChainGate, Priority, Status, Task},
    Layout, Project,
};

#[derive(Default)]
struct Fake {
    launched: Mutex<Vec<Task>>,
}
impl Workers for Fake {
    fn live<'a>(&'a self, _: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
    fn ended<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
    fn launch<'a>(
        &'a self,
        _: &'a Project,
        task: &'a Task,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.launched.lock().unwrap().push(task.clone());
            Ok(())
        })
    }
    fn stop<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn relay<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn superagent_answer_starts_or_saves_when_archived() {
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::with_home(home.path().join("board"));
    let project = Project::create(&layout, root.path(), Some("test"), Some("TEST")).unwrap();
    let add_blocked = || {
        let id = commands::add(
            &layout,
            project.clone(),
            &Common::new(Actor::User, ChainGate::InReview),
            "question",
            None,
            Some(Status::Todo),
            Priority::Medium,
            &[],
            &[],
            &[],
            false,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let board = Board::open(&layout, project.clone()).unwrap();
        let mut task = board.get(&id).unwrap();
        task.status = Status::Blocked;
        task.push_activity(chrono::Utc::now(), Actor::System, "blocked: Proceed?");
        board.save(&task).unwrap();
        id
    };
    let first = add_blocked();
    let fake = Arc::new(Fake::default());
    let dispatcher = Dispatcher::acquire(
        layout.clone(),
        home.path().join("config.json"),
        fake.clone(),
    )
    .await
    .unwrap();
    let result = dispatcher
        .relay(&project.slug, &first, "Yes, proceed")
        .await
        .unwrap();
    assert_eq!(result["started"], true);
    let task = Board::open(&layout, project.clone())
        .unwrap()
        .get(&first)
        .unwrap();
    assert_eq!(task.status, Status::InProgress);
    assert!(task
        .activity
        .iter()
        .any(|a| a.actor == Actor::User && a.text.contains("Yes, proceed")));
    assert_eq!(fake.launched.lock().unwrap().len(), 1);

    let second = add_blocked();
    let mut archived = project.clone();
    archived.archived = true;
    archived.save(&layout).unwrap();
    let result = dispatcher
        .relay(&project.slug, &second, "Approved")
        .await
        .unwrap();
    assert_eq!(result["saved"], true);
    assert_eq!(result["error"]["code"], "E_PROJECT_ARCHIVED");
    let task = Board::open(&layout, project).unwrap().get(&second).unwrap();
    assert_eq!(task.status, Status::Todo);
    assert!(task.activity.iter().any(|a| a.text.contains("Approved")));
}
