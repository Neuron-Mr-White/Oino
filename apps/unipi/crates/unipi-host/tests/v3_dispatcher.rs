#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "support/oin31.rs"]
mod oin31;

use futures_util::future::BoxFuture;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use unipi_host::superagent::{Config, Dispatcher, Workers, INTERRUPTED};
use unipi_kanboard::{
    board::Board,
    commands::{self, Common},
    model::{Actor, ChainGate, Priority, Status, Task},
    Layout, Project,
};

#[test]
fn legacy_config_cap_is_accepted_and_not_serialized() {
    let config: Config = serde_json::from_str(r#"{"enabled":true,"concurrencyCap":1}"#).unwrap();
    assert!(config.enabled);
    assert!(serde_json::to_value(config).unwrap().get("concurrencyCap").is_none());
    assert!(serde_json::from_str::<Config>(r#"{"enabled":true}"#).is_ok());
}

#[derive(Default)]
struct Fake {
    live: Mutex<HashSet<String>>,
    launches: AtomicUsize,
    pending: AtomicBool,
    fail_relay: AtomicBool,
    background: AtomicBool,
    retirements: AtomicUsize,
    relays: Mutex<Vec<String>>,
    last: Mutex<Option<String>>,
    approvals: Mutex<Vec<oino_core::session::permission::ApprovalRequest>>,
    presets: Mutex<Vec<unipi_host::superagent_presets::Preset>>,
    batches: Mutex<Vec<Vec<String>>>,
    deliveries: Mutex<Vec<(String, unipi_host::superagent::Delivery)>>,
    modes: Mutex<Vec<String>>,
}
impl Workers for Fake {
    fn live<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.live.lock().unwrap().contains(id) })
    }
    fn launch<'a>(
        &'a self,
        _: &'a Project,
        _: &'a Task,
        id: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.launches.fetch_add(1, Ordering::SeqCst);
            if self.pending.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            self.live.lock().unwrap().insert(id.into());
            Ok(())
        })
    }
    fn launch_preset<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
        preset: &'a unipi_host::superagent_presets::Preset,
    ) -> BoxFuture<'a, Result<(), String>> {
        self.presets.lock().unwrap().push(preset.clone());
        self.launch(project, task, id)
    }
    fn launch_lead<'a>(&'a self, project: &'a Project, tasks: &'a [Task], id: &'a str, preset: &'a unipi_host::superagent_presets::Preset) -> BoxFuture<'a, Result<(), String>> {
        self.batches.lock().unwrap().push(tasks.iter().map(|t| t.id.clone()).collect());
        self.modes.lock().unwrap().push(preset.mode.clone());
        self.launch_preset(project, &tasks[0], id, preset)
    }
    fn relay_delivery<'a>(&'a self, id: &'a str, text: &'a str, delivery: unipi_host::superagent::Delivery) -> BoxFuture<'a, Result<(), String>> {
        self.deliveries.lock().unwrap().push((text.to_owned(), delivery));
        self.relay(id, text)
    }
    fn set_mode<'a>(&'a self, _: &'a str, mode: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { self.modes.lock().unwrap().push(mode.into()); Ok(()) })
    }
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.live.lock().unwrap().remove(id);
            Ok(())
        })
    }
    fn relay<'a>(&'a self, id: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.fail_relay.load(Ordering::SeqCst) {
                return Err("offline".into());
            }
            self.relays.lock().unwrap().push(id.into());
            Ok(())
        })
    }
    fn approvals<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Vec<oino_core::session::permission::ApprovalRequest>> {
        Box::pin(async { self.approvals.lock().unwrap().clone() })
    }
    fn approval_response<'a>(
        &'a self,
        _: &'a str,
        response: oino_core::session::permission::ApprovalResponse,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut approvals = self.approvals.lock().unwrap();
            let i = approvals
                .iter()
                .position(|a| a.request_id == response.request_id)
                .ok_or("stale")?;
            approvals.remove(i);
            Ok(())
        })
    }
    fn ended<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { if self.background.load(Ordering::SeqCst) { None } else { self.last.lock().unwrap().clone() } })
    }
    fn retire<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async { self.retirements.fetch_add(1, Ordering::SeqCst); })
    }
}
struct Fixture {
    home: tempfile::TempDir,
    _root: tempfile::TempDir,
    layout: Layout,
    project: Project,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::with_home(home.path().join("board"));
        let project = Project::create(&layout, root.path(), Some("test"), Some("TEST")).unwrap();
        Self {
            home,
            _root: root,
            layout,
            project,
        }
    }
    fn add(&self, title: &str, deps: &[String]) -> String {
        commands::add(
            &self.layout,
            self.project.clone(),
            &Common::new(Actor::User, ChainGate::InReview),
            title,
            None,
            Some(Status::Todo),
            Priority::Medium,
            deps,
            &[],
            &[],
            false,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn dispatcher(&self, fake: Arc<Fake>) -> Arc<Dispatcher> {
        Dispatcher::acquire(
            self.layout.clone(),
            self.home.path().join("config.json"),
            fake,
        )
        .await
        .unwrap()
    }
    fn task(&self, id: &str) -> Task {
        Board::open(&self.layout, self.project.clone())
            .unwrap()
            .get(id)
            .unwrap()
    }
}
#[tokio::test]
async fn dispatch_blockers_explain_manual_and_automatic_behavior() {
    let f = Fixture::new();
    let task = f.add("ready", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["registered"], true);
    let blockers = status["projects"][0]["blockers"].as_array().unwrap();
    assert_eq!(blockers[0]["code"], "project_auto_off");
    assert_eq!(blockers[1]["code"], "global_auto_off");
    assert_eq!(d.start("missing", &task).await.unwrap_err().code, "E_PROJECT_NOT_REGISTERED");
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
    d.start(&f.project.slug, &task).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn recent_workspaces_exclude_registered_and_archived_and_are_capped() {
    use unipi_host::sessions::card::ProjectGroup;
    let f = Fixture::new();
    let archived_root = tempfile::tempdir().unwrap();
    let mut archived = Project::create(&f.layout, archived_root.path(), Some("archived"), None).unwrap();
    archived.archived = true;
    archived.save(&f.layout).unwrap();
    let roots: Vec<_> = (0..7).map(|_| tempfile::tempdir().unwrap()).collect();
    let group = |path: &std::path::Path| ProjectGroup { cwd: path.to_string_lossy().into(), name: "workspace".into(), cards: vec![] };
    let mut recent = vec![group(&f.project.root), group(archived_root.path())];
    recent.extend(roots.iter().map(|root| group(root.path())));
    let d = f.dispatcher(Arc::new(Fake::default())).await;
    let mut status = d.status().await.unwrap();
    d.append_recent_workspaces(&mut status, &recent).unwrap();
    let rows = status["projects"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for row in &rows[1..] {
        assert_eq!(row["registered"], false);
        assert_eq!(row["blockers"][0]["code"], "not_registered");
        assert_eq!(row["blockers"][0]["action"]["kind"], "register");
    }
}

#[tokio::test]
async fn enable_actions_register_unarchive_and_enable_auto() {
    let f = Fixture::new();
    let d = f.dispatcher(Arc::new(Fake::default())).await;
    assert_eq!(d.blockers("missing").await.unwrap()[0].code, "not_registered");
    assert_eq!(d.enable("enable_project_auto", Some(&f.project.slug), None).await.unwrap()["ok"], true);
    assert_eq!(d.status().await.unwrap()["projects"][0]["autoStart"], true);
    assert_eq!(d.enable("enable_auto_start", None, None).await.unwrap()["projectId"], serde_json::Value::Null);
    assert_eq!(d.status().await.unwrap()["enabled"], true);
    let mut archived = f.project.clone();
    archived.archived = true;
    archived.save(&f.layout).unwrap();
    d.enable("unarchive", Some(&archived.slug), None).await.unwrap();
    assert!(!Project::load(&f.layout, &archived.slug).unwrap().archived);
    let root = tempfile::tempdir().unwrap();
    let result = d.enable("register", None, Some(root.path().to_str().unwrap())).await.unwrap();
    let created = Project::load(&f.layout, result["projectId"].as_str().unwrap()).unwrap();
    assert_eq!(created.root, root.path().canonicalize().unwrap());
    assert_eq!(d.enable("register", None, Some("relative")).await.unwrap_err().code, "E_SCHEMA");
    assert_eq!(d.enable("unknown", None, None).await.unwrap_err().code, "E_SCHEMA");
}

#[tokio::test]
async fn presets_persist_and_apply_overrides_to_launch() {
    use unipi_host::superagent_presets::Overrides;
    let f = Fixture::new();
    let task = f.add("preset", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let list = d.presets_list().await;
    assert_eq!(list["defaultPresetId"], "quick-fix");
    assert_eq!(list["presets"].as_array().unwrap().len(), 5);
    let mut custom = unipi_host::superagent_presets::Store::default()
        .resolve(Some("feature"), Overrides::default())
        .unwrap();
    custom.id = "custom".into();
    custom.model = Some("faux/example".into());
    custom.thinking = Some("high".into());
    d.preset_save(custom).await.unwrap();
    d.preset_set_default("custom").await.unwrap();
    assert!(d.preset_delete("custom").await.is_err());
    d.start_with_preset(
        &f.project.slug,
        &task,
        None,
        Overrides {
            autonomy: Some("low".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let preset = fake.presets.lock().unwrap()[0].clone();
    assert_eq!(preset.model.as_deref(), Some("faux/example"));
    assert_eq!(preset.thinking.as_deref(), Some("high"));
    assert_eq!(preset.autonomy, "low");
    assert_eq!(preset.mode, "graph");
    d.preset_set_default("quick-fix").await.unwrap();
    assert_eq!(d.preset_delete("custom").await.unwrap()["deleted"], true);
    drop(d);
    let reopened = f.dispatcher(fake).await;
    assert_eq!(
        reopened.presets_list().await["presets"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[tokio::test]
async fn watcher_requires_project_toggle_and_status_separates_review() {
    let f = Fixture::new();
    let task = f.add("ready", &[]);
    let review = f.add("review", &[]);
    let blocked = f.add("blocked", &[]);
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    let mut t = board.get(&review).unwrap();
    t.status = Status::InReview;
    board.save(&t).unwrap();
    let mut t = board.get(&blocked).unwrap();
    t.status = Status::Blocked;
    t.push_activity(
        chrono::Utc::now(),
        Actor::System,
        "blocked: Need a decision",
    );
    board.save(&t).unwrap();
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
    })
    .await
    .unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
    d.configure_project(&f.project.slug, true).await.unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["autoStart"], true);
    assert_eq!(status["leads"][0]["ticketIds"], serde_json::json!([task]));
    assert_eq!(status["leads"][0]["presetId"], "quick-fix");
    assert_eq!(
        status["leads"][0]["progress"],
        serde_json::json!({"running":1,"queued":0,"done":0})
    );
    assert_eq!(status["needsYou"].as_array().unwrap().len(), 1);
    assert_eq!(status["needsYou"][0]["question"], "Need a decision");
    assert_eq!(status["review"][0]["taskId"], review);
    drop(d);
    let reopened = f.dispatcher(fake).await;
    assert_eq!(
        reopened.status().await.unwrap()["projects"][0]["autoStart"],
        true
    );
}

#[tokio::test]
async fn archived_projects_and_tasks_are_absent_from_status_and_dispatch() {
    let f = Fixture::new();
    let archived_task = f.add("archived review", &[]);
    let review = f.add("visible review", &[]);
    let ready = f.add("ready", &[]);
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    for (id, status) in [(&archived_task, Status::Archived), (&review, Status::InReview)] {
        let mut task = board.get(id).unwrap();
        task.status = status;
        board.save(&task).unwrap();
    }
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let status = d.status().await.unwrap();
    assert_eq!(status["review"].as_array().unwrap().len(), 1);
    assert_eq!(status["review"][0]["taskId"], review);
    assert!(d.start(&f.project.slug, &archived_task).await.is_err());
    let mut project = f.project.clone();
    project.archived = true;
    project.save(&f.layout).unwrap();
    let status = d.status().await.unwrap();
    for key in ["projects", "review", "workers", "leads", "needsYou"] {
        assert!(status[key].as_array().unwrap().is_empty(), "{key}");
    }
    assert_eq!(d.blockers(&project.slug).await.unwrap()[0].code, "archived");
    assert_eq!(d.start(&project.slug, &ready).await.unwrap_err().code, "E_PROJECT_ARCHIVED");
    d.configure_project(&project.slug, true).await.unwrap();
    d.configure(Config { enabled: true, concurrency_cap: 1 }).await.unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn multiple_tasks_feed_one_lead_and_route_delivery() {
    use unipi_host::superagent::Delivery;
    let f = Fixture::new();
    let a = f.add("first", &[]); let b = f.add("second", &[]); let c = f.add("third", &[]);
    let fake = Arc::new(Fake::default()); let d = f.dispatcher(fake.clone()).await;
    let id = d.start_tasks(&f.project.slug, &[a.clone(), b.clone()], None, Default::default()).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"].as_array().unwrap().len(), 1);
    assert_eq!(status["leads"][0]["ticketIds"].as_array().unwrap().len(), 2);
    assert_eq!(fake.batches.lock().unwrap()[0], vec![a.clone(), b.clone()]);
    assert_eq!(f.task(&a).run.unwrap().session, id);
    assert_eq!(f.task(&b).run.unwrap().session, id);
    assert_eq!(d.start(&f.project.slug, &c).await.unwrap(), id);
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    assert_eq!(fake.deliveries.lock().unwrap()[0].1, Delivery::Queue);
    d.relay_to_lead(&f.project.slug, "complement", &[b.clone()], Delivery::Steer, true).await.unwrap();
    d.relay_to_lead(&f.project.slug, "dependent", &[c.clone()], Delivery::Queue, false).await.unwrap();
    let deliveries = fake.deliveries.lock().unwrap();
    assert_eq!(deliveries[1].1, Delivery::Steer);
    assert!(deliveries[1].0.starts_with("URGENT: schedule this first / add it as the first graph item"));
    assert_eq!(deliveries[2], ("dependent".into(), Delivery::Queue));
    assert!(f.task(&b).activity.iter().any(|a| a.text.contains("[superagent]") && a.text.contains("complement")));
    drop(deliveries);
    let root = tempfile::tempdir().unwrap();
    let other = Project::create(&f.layout, root.path(), Some("other"), Some("OT")).unwrap();
    let task = commands::add(&f.layout, other.clone(), &Common::new(Actor::User, ChainGate::InReview), "other", None, Some(Status::Todo), Priority::Medium, &[], &[], &[], false).unwrap();
    assert_ne!(d.start(&other.slug, task["id"].as_str().unwrap()).await.unwrap(), id);
}

#[tokio::test]
async fn auto_start_batches_all_ready_tickets() {
    let f = Fixture::new(); let a = f.add("first", &[]); let b = f.add("second", &[]); let _blocked = f.add("wait", std::slice::from_ref(&a));
    let fake = Arc::new(Fake::default()); let d = f.dispatcher(fake.clone()).await;
    d.configure(Config { enabled: true, concurrency_cap: 1 }).await.unwrap(); d.configure_project(&f.project.slug, true).await.unwrap();
    d.tick().await.unwrap();
    let batches = fake.batches.lock().unwrap(); assert_eq!(batches.len(), 1); assert_eq!(batches[0].len(), 2); assert!(batches[0].contains(&a)); assert!(batches[0].contains(&b));
}

#[tokio::test]
async fn preset_mode_and_switch_reach_lead_and_status() {
    let f = Fixture::new(); let task = f.add("mode", &[]);
    let fake = Arc::new(Fake::default()); let d = f.dispatcher(fake.clone()).await;
    let id = d.start_tasks(&f.project.slug, &[task], Some("feature"), Default::default()).await.unwrap();
    assert_eq!(fake.modes.lock().unwrap().as_slice(), ["graph"]);
    assert_eq!(d.status().await.unwrap()["leads"][0]["mode"], "graph");
    assert_eq!(d.set_mode(&f.project.slug, "swarm").await.unwrap(), serde_json::json!({"sessionId":id,"mode":"swarm"}));
    assert_eq!(fake.modes.lock().unwrap().as_slice(), ["graph", "swarm"]);
    assert_eq!(d.status().await.unwrap()["leads"][0]["mode"], "swarm");
    assert!(d.set_mode(&f.project.slug, "invalid").await.is_err());
}

#[tokio::test]
async fn completed_lead_retires_and_new_work_starts_fresh() {
    let f = Fixture::new();
    let a = f.add("review", &[]);
    let b = f.add("blocked", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start_tasks(&f.project.slug, &[a.clone(), b.clone()], None, Default::default()).await.unwrap();
    commands::finish(&f.layout, f.project.clone(), ChainGate::InReview, &a, &lead, "done", &[], chrono::Utc::now()).unwrap();
    unipi_kanboard::dispatcher::approval_transition(&f.layout, f.project.clone(), &b, &lead, Some("Which option?")).unwrap();
    *fake.last.lock().unwrap() = Some("last message".into());
    d.tick().await.unwrap();
    assert_eq!(f.task(&a).status, Status::InReview);
    assert_eq!(f.task(&b).status, Status::Blocked);
    assert!(f.task(&b).run.is_none());
    assert_eq!(d.status().await.unwrap()["needsYou"][0]["question"], "Which option?");
    *fake.last.lock().unwrap() = None;
    let c = f.add("fresh", &[]);
    assert_ne!(d.start(&f.project.slug, &c).await.unwrap(), lead);
    assert_eq!(fake.launches.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ended_lead_blocks_every_in_progress_ticket_with_last_message() {
    let f = Fixture::new(); let a = f.add("a", &[]); let b = f.add("b", &[]);
    let fake = Arc::new(Fake::default()); let d = f.dispatcher(fake.clone()).await;
    d.start_tasks(&f.project.slug, &[a.clone(), b.clone()], None, Default::default()).await.unwrap();
    *fake.last.lock().unwrap() = Some("Need clarification".into()); d.tick().await.unwrap(); d.tick().await.unwrap();
    for id in [a, b] { let t = f.task(&id); assert_eq!(t.status, Status::Blocked); assert!(t.run.is_none()); assert!(t.activity.last().unwrap().text.contains("Need clarification")); }
}

#[tokio::test]
async fn idle_lead_with_background_graph_is_not_retired() {
    let f = Fixture::new();
    let task = f.add("graph", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &task).await.unwrap();
    *fake.last.lock().unwrap() = Some(String::new());
    fake.background.store(true, Ordering::SeqCst);
    d.tick().await.unwrap();
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(fake.retirements.load(Ordering::SeqCst), 0);
    assert!(fake.deliveries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn idle_lead_gets_one_continuation_then_is_blocked() {
    let f = Fixture::new();
    let task = f.add("idle", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &task).await.unwrap();
    *fake.last.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(fake.deliveries.lock().unwrap().len(), 1);
    assert!(fake.deliveries.lock().unwrap()[0].0.contains(&task));
    // Reacquisition preserves the consumed budget.
    drop(d);
    let d = f.dispatcher(fake.clone()).await;
    d.tick().await.unwrap();
    assert_eq!(fake.deliveries.lock().unwrap().len(), 1);
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert!(f.task(&task).activity.last().unwrap().text.contains("Worker ended without a transition; move to todo to retry"));
}

#[tokio::test]
async fn graph_continuation_names_remaining_tickets_and_progress_replenishes_budget() {
    let f = Fixture::new();
    let a = f.add("a", &[]); let b = f.add("b", &[]); let c = f.add("c", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start_tasks(&f.project.slug, &[a.clone(), b.clone(), c.clone()], None, Default::default()).await.unwrap();
    d.set_mode(&f.project.slug, "graph").await.unwrap();
    assert_eq!(fake.modes.lock().unwrap().last().unwrap(), "graph");
    d.tick().await.unwrap(); // record initial claims while working
    commands::finish(&f.layout, f.project.clone(), ChainGate::InReview, &a, &lead, "done", &[], chrono::Utc::now()).unwrap();
    *fake.last.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    let text = fake.deliveries.lock().unwrap()[0].0.clone();
    assert!(!text.contains(&a)); assert!(text.contains(&b)); assert!(text.contains(&c));
    commands::finish(&f.layout, f.project.clone(), ChainGate::InReview, &b, &lead, "done", &[], chrono::Utc::now()).unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.deliveries.lock().unwrap().len(), 2);
    assert!(fake.deliveries.lock().unwrap()[1].0.contains(&c));
    d.tick().await.unwrap();
    assert_eq!(f.task(&c).status, Status::Blocked);
}

#[tokio::test]
async fn parked_claim_protocol_stop_and_restart() {
    use oino_core::session::permission::{
        ApprovalDecision, ApprovalRequest, ApprovalResponse, Level,
    };
    let f = Fixture::new();
    let id = f.add("park", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let session = d.start(&f.project.slug, &id).await.unwrap();
    let request = ApprovalRequest {
        request_id: "request".into(),
        session_id: "child".into(),
        tool: "bash".into(),
        arguments: serde_json::from_value(
            serde_json::json!({"command":"git push", "summary":"push"}),
        )
        .unwrap(),
        risk_level: Some(Level::High),
        ceiling: Level::Medium,
        reason: "needs Autonomy High".into(),
    };
    fake.approvals.lock().unwrap().push(request);
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &id,
        &session,
        Some("git push risk high ceiling medium needs approval"),
    )
    .unwrap();
    d.tick().await.unwrap();
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert_eq!(f.task(&id).run.unwrap().session, session);
    let item = d.status().await.unwrap()["needsYou"][0].clone();
    assert_eq!(
        item,
        serde_json::json!({"projectId":f.project.slug,"taskId":id,"status":"blocked","workerSessionId":session,"requestId":"request","sessionId":"child","tool":"bash","command":"git push","summary":"push","risk":"high","ceiling":"medium","reason":"needs Autonomy High"})
    );
    let response = ApprovalResponse {
        request_id: "request".into(),
        decision: ApprovalDecision::Deny,
    };
    assert!(d
        .approval_response(&f.project.slug, &id, "wrong", response.clone())
        .await
        .is_err());
    assert!(d
        .approval_response(
            &f.project.slug,
            &id,
            &session,
            ApprovalResponse {
                request_id: "stale".into(),
                ..response.clone()
            }
        )
        .await
        .is_err());
    d.approval_response(&f.project.slug, &id, &session, response.clone())
        .await
        .unwrap();
    assert!(d
        .approval_response(&f.project.slug, &id, &session, response)
        .await
        .is_err());
    assert!(d.stop(&f.project.slug, &id).await.unwrap());
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert!(f.task(&id).run.is_none());

    let another = f.add("restart", &[]);
    let session = d.start(&f.project.slug, &another).await.unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &another,
        &session,
        Some("pending"),
    )
    .unwrap();
    fake.live.lock().unwrap().remove(&session);
    drop(d);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _restarted = f.dispatcher(fake).await;
    let task = f.task(&another);
    assert_eq!(task.status, Status::Blocked);
    assert!(task.run.is_none());
    assert!(task.activity.last().unwrap().text.contains(INTERRUPTED));
}

#[tokio::test]
async fn watcher_manual_race_one_run() {
    let f = Fixture::new();
    let id = f.add("race", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: true,
        concurrency_cap: 3,
    })
    .await
    .unwrap();
    d.configure_project(&f.project.slug, true).await.unwrap();
    let (_, _) = tokio::join!(d.tick(), d.start(&f.project.slug, &id));
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    assert!(f.task(&id).run.is_some());
}
#[tokio::test]
async fn two_todos_one_project_worker() {
    let f = Fixture::new();
    let a = f.add("a", &[]);
    let b = f.add("b", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: false,
        concurrency_cap: 3,
    })
    .await
    .unwrap();
    let (a, b) = tokio::join!(d.start(&f.project.slug, &a), d.start(&f.project.slug, &b));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn crash_mid_launch_blocks_then_manual_restart_starts_fresh() {
    let f = Fixture::new();
    let id = f.add("crash", &[]);
    let fake = Arc::new(Fake::default());
    fake.pending.store(true, Ordering::SeqCst);
    let d = f.dispatcher(fake.clone()).await;
    let launch = {
        let d = d.clone();
        let project = f.project.slug.clone();
        let id = id.clone();
        tokio::spawn(async move { d.start(&project, &id).await })
    };
    while fake.launches.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let reserved = f.task(&id).run.unwrap().session;
    assert!(!reserved.is_empty());
    launch.abort();
    let _ = launch.await;
    drop(d);
    // A concurrent subprocess test may briefly inherit an fd between fork and exec.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let restarted = f.dispatcher(fake.clone()).await;
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert!(f
        .task(&id)
        .activity
        .last()
        .unwrap()
        .text
        .contains(INTERRUPTED));
    // Manual start takes blocked tickets, so the interrupted reservation is replaced.
    fake.pending.store(false, Ordering::SeqCst);
    let retried = restarted.start(&f.project.slug, &id).await.unwrap();
    assert_ne!(retried, reserved);
    assert_eq!(fake.launches.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn advisory_owner_cap_dependencies_and_relay() {
    let f = Fixture::new();
    let dep = f.add("dependency", &[]);
    let id = f.add("dependent", std::slice::from_ref(&dep));
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    assert!(Dispatcher::acquire(
        f.layout.clone(),
        f.home.path().join("config.json"),
        fake.clone()
    )
    .await
    .is_err());
    assert!(d.start(&f.project.slug, &id).await.is_err());
    let session = d.start(&f.project.slug, &dep).await.unwrap();
    assert!(d.start(&f.project.slug, &id).await.is_err());
    let reply = d.relay(&f.project.slug, &dep, "hello").await.unwrap();
    assert_eq!(reply["sessionId"], session);
    assert_eq!(reply["delivered"], true);
    assert_eq!(*fake.relays.lock().unwrap(), vec![session]);
    fake.fail_relay.store(true, Ordering::SeqCst);
    assert_eq!(
        d.relay(&f.project.slug, &dep, "persist me").await.unwrap()["delivered"],
        false
    );
    assert!(f
        .task(&dep)
        .activity
        .last()
        .unwrap()
        .text
        .contains("persist me"));
    assert!(d.relay(&f.project.slug, &id, "wrong worker").await.is_err());
}
#[cfg(windows)]
#[tokio::test]
async fn windows_tray_dispatcher() {
    let f = Fixture::new();
    let id = f.add("tray", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    assert!(Dispatcher::acquire(
        f.layout.clone(),
        f.home.path().join("config.json"),
        fake.clone()
    )
    .await
    .is_err());
    d.start(&f.project.slug, &id).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn superagent_has_only_cross_project_tools_and_no_cwd() {
    use oino_core::{
        extensions::api::ExtensionMode,
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
    };
    use unipi_host::sessions::{HostHubOptions, SessionHub};
    let dir = tempfile::tempdir().unwrap();
    let memory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../extensions/dist/unipi.memory");
    assert!(memory.join("component.wasm").is_file());
    let extensions = dir.path().join("agent/extensions");
    std::fs::create_dir_all(&extensions).unwrap();
    std::os::unix::fs::symlink(&memory, extensions.join("unipi.memory")).unwrap();
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().into(),
        agent_dir: dir.path().join("agent"),
        docs_root: None,
        session_dir: Some(dir.path().join("sessions")),
        mode: ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..Default::default()
        },
        loader: Arc::new(StaticLoader(Vec::new())),
        models: Some(oino_models::ModelRegistry::in_memory(Arc::new(
            oino_models::MapEnv::default(),
        ))),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    let runtime = unipi_host::superagent_session::create(hub.clone())
        .await
        .unwrap();
    let session = runtime.session();
    assert_eq!(session.cwd(), dir.path().join("agent/superagent"));
    assert!(session.active_tool_names().contains(&"memory_search".to_owned()));
    assert!(session.active_tool_names().contains(&"memory_store".to_owned()));
    let tools = session.all_tools();
    oin31::audit_and_wire(&tools);
    for name in ["superagent.start", "superagent_start", "superagent.set_mode", "superagent_set_mode"] {
        let args = serde_json::from_value(serde_json::json!({"projectId":"test", "taskIds":["T-1"], "mode":"goal"})).unwrap();
        assert!(session.check_permission(name, &args).await.is_ok(), "permission denied: {name}");
    }
    let resolver = tools.iter().find(|t| t.name == "project_resolver").unwrap();
    let call: oino_ai::ToolCall = serde_json::from_value(serde_json::json!({"id":"call_1","name":"project_resolver","arguments":{"reason":"find project"}})).unwrap();
    assert!(oino_agent::validate_tool_arguments(&resolver.parameters, &call).is_ok());
    let empty: oino_ai::ToolCall = serde_json::from_value(serde_json::json!({"id":"call_2","name":"project_resolver","arguments":{}})).unwrap();
    assert!(oino_agent::validate_tool_arguments(&resolver.parameters, &empty).is_ok());
    let mut names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    names.sort();
    for required in ["ask_user", "kanboard", "project_resolver", "superagent_start", "superagent_set_mode", "memory_search", "memory_store"] { assert!(names.iter().any(|n| n == required), "missing {required}: {names:?}"); }
    for forbidden in ["read", "write", "edit", "bash", "grep", "find", "ls"] { assert!(!names.iter().any(|n| n == forbidden), "unexpected {forbidden}"); }
    let shell =
        serde_json::from_value(serde_json::json!({"command":"ls","riskLevel":"low"})).unwrap();
    assert!(session
        .check_permission("bash", &shell)
        .await
        .unwrap_err()
        .contains("no workspace"));
    assert!(session.change_cwd(dir.path().into()).await.is_err());
    let file = session.with_manager(|m| m.session_file().unwrap().to_path_buf());
    assert!(unipi_host::superagent_session::is_superagent_file(&file));
    let id = session.session_id();
    // Opening the visible chat shares this exact runtime and does not steal its lease.
    let opened = hub.open(&id).await.unwrap();
    assert_eq!(opened.session_id(), id);
    runtime.dispose().await;
    let resumed = unipi_host::superagent_session::create_from(hub, Some(file))
        .await
        .unwrap();
    assert_eq!(resumed.session().cwd(), dir.path().join("agent/superagent"));
    assert!(resumed.session().active_tool_names().contains(&"memory_search".to_owned()));
    resumed.dispose().await;
}

#[tokio::test]
async fn multiple_projects_dispatch_without_a_global_limit() {
    let f = Fixture::new();
    let first = f.add("oino work", &[]);
    let roots: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let projects: Vec<_> = ["zmstudio-migration", "cloud-agent", "manual"].iter().enumerate().map(|(i, name)|
        Project::create(&f.layout, roots[i].path(), Some(name), Some(&format!("P{i}"))).unwrap()).collect();
    let add = |project: &Project, title: &str| commands::add(&f.layout, project.clone(), &Common::new(Actor::User, ChainGate::InReview), title, None, Some(Status::Todo), Priority::Medium, &[], &[], &[], false).unwrap()["id"].as_str().unwrap().to_owned();
    let zm = add(&projects[0], "zm work");
    for i in 0..8 { add(&projects[1], &format!("cloud {i}")); }
    let manual = add(&projects[2], "manual work");
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &first).await.unwrap();
    d.start(&projects[0].slug, &zm).await.unwrap();
    d.configure(Config { enabled: true, concurrency_cap: 1 }).await.unwrap();
    d.configure_project(&projects[1].slug, true).await.unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.live.lock().unwrap().len(), 3);
    assert_eq!(fake.batches.lock().unwrap().last().unwrap().len(), 8);
    d.start(&projects[2].slug, &manual).await.unwrap();
    assert_eq!(fake.live.lock().unwrap().len(), 4);
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"].as_array().unwrap().len(), 4);
    assert!(status.get("concurrencyCap").is_none());
}

#[tokio::test]
async fn concurrent_projects_and_worker_exit_question() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let other_root = tempfile::tempdir().unwrap();
    let project =
        Project::create(&f.layout, other_root.path(), Some("other"), Some("OTHER")).unwrap();
    let b = commands::add(
        &f.layout,
        project.clone(),
        &Common::new(Actor::User, ChainGate::InReview),
        "second",
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
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &a).await.unwrap();
    d.start(&project.slug, &b).await.unwrap();
    assert_eq!(
        d.status().await.unwrap()["workers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(fake.live.lock().unwrap().len(), 2);
    *fake.last.lock().unwrap() = Some("Which API key should I use?".into());
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    assert_eq!(f.task(&a).status, Status::Blocked);
    assert!(f
        .task(&a)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Which API key should I use?"));
    assert_eq!(
        Board::open(&f.layout, project)
            .unwrap()
            .get(&b)
            .unwrap()
            .status,
        Status::Blocked
    );
}

#[tokio::test]
async fn real_worker_uses_reserved_id_unattended_ceiling_and_author_relay() {
    use oino_core::{
        extensions::api::{ExtensionInstance, ExtensionMode},
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
        testing::{FauxComponent, FauxProvider, FauxResponse},
    };
    use unipi_host::{
        sessions::{HostHubOptions, SessionHub},
        superagent::HostWorkers,
    };
    let f = Fixture::new();
    let id = f.add("real worker", &[]);
    let provider = FauxProvider::new(vec![FauxResponse::Text(
        "Need the API key before continuing".into(),
    )]);
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: f.project.root.clone(),
        agent_dir: f.home.path().join("agent"),
        docs_root: None,
        session_dir: Some(f.home.path().join("sessions")),
        mode: ExtensionMode::Desktop,
        has_ui: true,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..Default::default()
        },
        loader: Arc::new(StaticLoader(vec![
            Arc::new(FauxComponent(provider.clone())) as Arc<dyn ExtensionInstance>,
        ])),
        models: Some(models),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    let workers = HostWorkers::new(hub.clone(), f.layout.clone());
    let mut events = hub.subscribe();
    let session_id = uuid::Uuid::now_v7().to_string();
    let task = unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &id, &session_id)
        .unwrap();
    let mut preset = unipi_host::superagent_presets::Store::default()
        .resolve(Some("research"), Default::default())
        .unwrap();
    preset.model = Some("faux/faux-1".into());
    preset.thinking = Some("off".into());
    workers
        .launch_preset(&f.project, &task, &session_id, &preset)
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, unipi_host::sessions::hub::HubEvent::SessionChanged { card } if card.session_id == session_id && card.worker.is_some())
    );
    let card = hub
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.session_id == session_id)
        .unwrap();
    let worker = card.worker.unwrap();
    assert_eq!(worker.task_id, id);
    assert_eq!(worker.project_id, f.project.slug);
    let path = card.file;
    assert!(path.exists());
    let runtime = unipi_host::superagent_session::runtime(&session_id).unwrap();
    let session = runtime.session();
    session.wait_for_idle().await;
    assert!(path.exists(), "worker transcript remains after end");
    assert_eq!(session.session_id(), session_id);
    assert_eq!(session.model().unwrap().id, "faux-1");
    assert_eq!(session.thinking_level().to_string(), "off");
    assert_eq!(
        session.autonomy(),
        oino_core::session::permission::Level::Medium
    );
    session.set_approval_ui(true); // sticky unattended cannot be undone by opening the UI
    let marker = f.home.path().join("allowed-ran");
    let command = format!("touch {}", marker.display());
    let agent = session.clone();
    let call = tokio::spawn(async move { agent.execute_bash(command, false, None).await });
    while session.pending_approvals().is_empty() {
        tokio::task::yield_now().await;
    }
    assert!(!call.is_finished());
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert!(f.task(&id).run.is_some());
    let request = session.pending_approvals().pop().unwrap();
    session
        .approval_response(oino_core::session::permission::ApprovalResponse {
            request_id: request.request_id,
            decision: oino_core::session::permission::ApprovalDecision::AllowOnce,
        })
        .unwrap();
    assert!(call.await.unwrap().is_ok());
    assert!(marker.exists());
    assert_eq!(f.task(&id).status, Status::InProgress);
    let agent = session.clone();
    let denied = tokio::spawn(async move {
        agent
            .check_permission(
                "bash",
                &serde_json::from_value(
                    serde_json::json!({"command":"git push","riskLevel":"high"}),
                )
                .unwrap(),
            )
            .await
    });
    while session.pending_approvals().is_empty() {
        tokio::task::yield_now().await;
    }
    let request = session.pending_approvals().pop().unwrap();
    session
        .approval_response(oino_core::session::permission::ApprovalResponse {
            request_id: request.request_id,
            decision: oino_core::session::permission::ApprovalDecision::Deny,
        })
        .unwrap();
    assert_eq!(
        denied.await.unwrap().unwrap_err(),
        "blocked by permission (user denied)"
    );
    assert_eq!(f.task(&id).status, Status::InProgress);
    workers
        .launch(&f.project, &task, &session_id)
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 1); // attach is not another execution
    workers
        .relay(&session_id, "Use the existing test fixture")
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert!(session.entries().iter().any(|e| matches!(&e.kind, oino_session::EntryKind::CustomMessage { custom_type, details:Some(details), .. } if custom_type == "oino:superagent-relay" && details["author"] == "superagent")));
    let marker = f.home.path().join("must-not-run");
    let command = format!("touch {}", marker.display());
    let agent = session.clone();
    let cancelled = tokio::spawn(async move { agent.execute_bash(command, false, None).await });
    while session.pending_approvals().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(f.task(&id).status, Status::Blocked);
    // An open chat follows the worker through a backend over its runtime (host open path):
    // stopping must end that stream, or the chat's feed keeps tapping a dead runtime.
    let chat = oino_client::in_process::InProcessBackend::new(Arc::clone(&runtime));
    let mut chat_events = oino_client::SessionBackend::events(&chat);
    workers.stop(&session_id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(event) = futures_util::StreamExt::next(&mut chat_events).await {
            if matches!(event, oino_core::events::ClientEvent::Oino(oino_core::events::OinoEvent::SessionEvicted { .. })) {
                return;
            }
        }
        panic!("chat stream ended without SessionEvicted");
    })
    .await
    .expect("stopping a worker did not end an open chat's stream");
    assert!(path.exists(), "worker transcript remains after stop");
    assert!(cancelled.await.unwrap().is_err());
    assert!(!marker.exists());
    let restarted = Dispatcher::acquire(
        f.layout.clone(),
        f.home.path().join("config.json"),
        Arc::new(HostWorkers::new(hub.clone(), f.layout.clone())),
    )
    .await
    .unwrap();
    assert!(
        path.exists(),
        "restart reconciliation preserves worker transcript"
    );
    drop(restarted);
    assert!(f.task(&id).run.is_none());
    assert_eq!(f.task(&id).status, Status::Blocked);
}

#[test]
fn dispatcher_owner_child() {
    let Some(home) = std::env::var_os("OINO_DISPATCH_TEST_HOME") else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let layout = Layout::with_home(home);
        let fake = Arc::new(Fake::default());
        fake.pending.store(true, Ordering::SeqCst);
        let d = Dispatcher::acquire(
            layout,
            std::env::var_os("OINO_DISPATCH_TEST_CONFIG")
                .unwrap()
                .into(),
            fake,
        )
        .await
        .unwrap();
        d.start(
            &std::env::var("OINO_DISPATCH_TEST_PROJECT").unwrap(),
            &std::env::var("OINO_DISPATCH_TEST_TASK").unwrap(),
        )
        .await
        .unwrap();
    });
}
#[tokio::test]
async fn process_crash_releases_os_lock_and_blocks_durable_reservation() {
    let f = Fixture::new();
    let id = f.add("process crash", &[]);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "dispatcher_owner_child", "--nocapture"])
        .env("OINO_DISPATCH_TEST_HOME", &f.layout.home)
        .env(
            "OINO_DISPATCH_TEST_CONFIG",
            f.home.path().join("config.json"),
        )
        .env("OINO_DISPATCH_TEST_PROJECT", &f.project.slug)
        .env("OINO_DISPATCH_TEST_TASK", &id)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while f.task(&id).run.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not reserve a session");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let fake = Arc::new(Fake::default());
    assert!(Dispatcher::acquire(
        f.layout.clone(),
        f.home.path().join("config.json"),
        fake.clone()
    )
    .await
    .is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    let d = f.dispatcher(fake.clone()).await;
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert!(f
        .task(&id)
        .activity
        .last()
        .unwrap()
        .text
        .contains(INTERRUPTED));
    // Manual start takes blocked tickets; the interrupted reservation is replaced.
    assert!(!d.start(&f.project.slug, &id).await.unwrap().is_empty());
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn status_reports_only_live_leads() {
    let f = Fixture::new();
    let task = f.add("lead", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let id = d.start(&f.project.slug, &task).await.unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["lead"], id);
    assert_eq!(status["leads"].as_array().unwrap().len(), 1);
    fake.live.lock().unwrap().remove(&id);
    let status = d.status().await.unwrap();
    assert!(status["projects"][0]["lead"].is_null());
    assert!(status["leads"].as_array().unwrap().is_empty());
    fake.live.lock().unwrap().insert(id);
    *fake.last.lock().unwrap() = Some("ended".into());
    let status = d.status().await.unwrap();
    assert!(status["projects"][0]["lead"].is_null());
    assert!(status["leads"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn status_counts_ready_tickets_using_dispatch_rule() {
    let f = Fixture::new();
    let first = f.add("ready", &[]);
    f.add("blocked dependency", &[first.clone()]);
    f.add("also ready", &[]);
    let d = f.dispatcher(Arc::new(Fake::default())).await;
    assert_eq!(d.status().await.unwrap()["projects"][0]["ready"], 2);
    d.start(&f.project.slug, &first).await.unwrap();
    assert_eq!(d.status().await.unwrap()["projects"][0]["ready"], 1);
}

#[tokio::test]
async fn manual_start_takes_backlog_and_blocked_with_requested_preset() {
    use unipi_host::superagent_presets::Overrides;
    let f = Fixture::new();
    let backlog = f.add("backlog", &[]);
    let blocked = f.add("blocked", &[]);
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&backlog).unwrap();
    task.status = Status::Backlog;
    board.save(&task).unwrap();
    let mut task = board.get(&blocked).unwrap();
    task.status = Status::Blocked;
    board.save(&task).unwrap();
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let first = d
        .start_with_preset(
            &f.project.slug,
            &backlog,
            None,
            Overrides {
                model: Some("faux/backlog".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        fake.presets.lock().unwrap()[0].model.as_deref(),
        Some("faux/backlog")
    );
    assert_eq!(f.task(&backlog).status, Status::InProgress);
    assert_eq!(f.task(&backlog).run.unwrap().session, first);
    assert!(d.stop(&f.project.slug, &backlog).await.unwrap());
    let second = d
        .start_with_preset(
            &f.project.slug,
            &blocked,
            None,
            Overrides {
                model: Some("faux/blocked".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        fake.presets.lock().unwrap()[1].model.as_deref(),
        Some("faux/blocked")
    );
    assert_eq!(f.task(&blocked).status, Status::InProgress);
}

#[tokio::test]
async fn watcher_tick_does_not_start_backlog() {
    let f = Fixture::new();
    let backlog = f.add("backlog", &[]);
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&backlog).unwrap();
    task.status = Status::Backlog;
    board.save(&task).unwrap();
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
    })
    .await
    .unwrap();
    d.configure_project(&f.project.slug, true).await.unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
    assert_eq!(f.task(&backlog).status, Status::Backlog);
}

#[tokio::test]
async fn manual_start_refuses_different_preset_while_lead_runs() {
    use unipi_host::superagent_presets::Overrides;
    let f = Fixture::new();
    let a = f.add("a", &[]);
    let b = f.add("b", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let id = d
        .start_with_preset(
            &f.project.slug,
            &a,
            None,
            Overrides {
                model: Some("faux/first".into()),
                thinking: Some("low".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let err = d
        .start_with_preset(
            &f.project.slug,
            &b,
            None,
            Overrides {
                model: Some("faux/second".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_LEAD_RUNNING");
    assert!(err.message.contains(&id));
    assert!(err.message.contains("superagent_stop"));
    assert_eq!(f.task(&b).status, Status::Todo);
    assert!(fake.relays.lock().unwrap().is_empty());
    assert!(fake.deliveries.lock().unwrap().is_empty());
    assert_eq!(
        d.start_with_preset(
            &f.project.slug,
            &b,
            None,
            Overrides {
                model: Some("faux/first".into()),
                thinking: Some("low".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
        id
    );
    assert_eq!(*fake.relays.lock().unwrap(), vec![id]);
}

#[tokio::test]
async fn project_lead_applies_to_watcher_launch_and_status() {
    let f = Fixture::new();
    let _task = f.add("lead", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
    })
    .await
    .unwrap();
    let saved = d
        .configure_project_lead(
            &f.project.slug,
            Some("faux/lead".into()),
            Some("max".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"projectId": f.project.slug.as_str(), "model": "faux/lead", "thinking": "max"})
    );
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["leadModel"], "faux/lead");
    assert_eq!(status["projects"][0]["leadThinking"], "max");
    assert_eq!(
        status["projects"][0]["root"],
        f.project.root.to_string_lossy().as_ref()
    );
    d.configure_project(&f.project.slug, true).await.unwrap();
    d.tick().await.unwrap();
    let preset = fake.presets.lock().unwrap()[0].clone();
    assert_eq!(preset.model.as_deref(), Some("faux/lead"));
    assert_eq!(preset.thinking.as_deref(), Some("max"));
}

#[tokio::test]
async fn explicit_override_wins_over_project_lead() {
    use unipi_host::superagent_presets::Overrides;
    let f = Fixture::new();
    let task = f.add("lead override", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure_project_lead(
        &f.project.slug,
        Some("faux/lead".into()),
        Some("low".into()),
    )
    .await
    .unwrap();
    d.start_with_preset(
        &f.project.slug,
        &task,
        None,
        Overrides {
            model: Some("faux/explicit".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let preset = fake.presets.lock().unwrap()[0].clone();
    assert_eq!(preset.model.as_deref(), Some("faux/explicit"));
    assert_eq!(preset.thinking.as_deref(), Some("low"));
}

#[tokio::test]
async fn project_lead_validation_and_clearing() {
    let f = Fixture::new();
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake).await;
    assert_eq!(
        d.configure_project_lead(
            &f.project.slug,
            Some("faux/lead".into()),
            Some("extreme".into())
        )
        .await
        .unwrap_err()
        .code,
        "E_SCHEMA"
    );
    assert_eq!(
        d.configure_project_lead(&f.project.slug, Some("not-a-model".into()), None)
            .await
            .unwrap_err()
            .code,
        "E_SCHEMA"
    );
    assert!(d
        .configure_project_lead("missing", Some("faux/lead".into()), None)
        .await
        .is_err());
    d.configure_project_lead(
        &f.project.slug,
        Some("faux/lead".into()),
        Some("high".into()),
    )
    .await
    .unwrap();
    let cleared = d
        .configure_project_lead(&f.project.slug, Some("  ".into()), None)
        .await
        .unwrap();
    assert_eq!(
        cleared,
        serde_json::json!({"projectId": f.project.slug.as_str(), "model": null, "thinking": null})
    );
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["leadModel"], serde_json::Value::Null);
    assert_eq!(
        status["projects"][0]["leadThinking"],
        serde_json::Value::Null
    );
}

#[test]
fn preferences_without_project_leads_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("superagent-preferences.json");
    std::fs::write(
        &path,
        r#"{"defaultPresetId":"quick-fix","presets":[],"projects":{}}"#,
    )
    .unwrap();
    let store = unipi_host::superagent_presets::Store::load(&path).unwrap();
    assert!(store.project_leads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unattached_lead_tracks_waking_but_not_quiet_work() {
    use oino_core::{
        extensions::api::{ExtensionInstance, ExtensionMode, Registration},
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
        testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse},
    };
    use unipi_host::{sessions::{HostHubOptions, SessionHub}, superagent::HostWorkers};
    let f = Fixture::new();
    let id = f.add("background lead", &[]);
    let wait = Arc::new(Mutex::new(Some("subagent working".to_owned())));
    let source = {
        let wait = Arc::clone(&wait);
        FakeExtension::new("waiter", Registration::default()).on_wait(move |_| wait.lock().unwrap().clone())
    };
    let provider = FauxProvider::new(vec![FauxResponse::Text("yielded".into())]);
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: f.project.root.clone(), agent_dir: f.home.path().join("agent"), docs_root: None,
        session_dir: Some(f.home.path().join("sessions")), mode: ExtensionMode::Desktop,
        has_ui: true, approve: Some(false), flags: ResourceFlags { no_context_files: true, ..Default::default() },
        loader: Arc::new(StaticLoader(vec![Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>, Arc::new(source)])),
        models: Some(models), tools: ToolSelection::Default, exclude_tools: Vec::new(), services_cap: None, host_data_dir: None,
    }).unwrap();
    let workers = Arc::new(HostWorkers::new(hub.clone(), f.layout.clone()));
    let session_id = uuid::Uuid::now_v7().to_string();
    let task = unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &id, &session_id).unwrap();
    let mut preset = unipi_host::superagent_presets::Store::default().resolve(Some("research"), Default::default()).unwrap();
    preset.model = Some("faux/faux-1".into());
    workers.launch_preset(&f.project, &task, &session_id, &preset).await.unwrap();
    let session = unipi_host::superagent_session::runtime(&session_id).unwrap().session();
    session.wait_for_idle().await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let card = hub.list().await.unwrap().into_iter().find(|c| c.session_id == session_id).unwrap();
    assert!(card.state.is_working(), "{card:?}");
    assert_eq!(card.pending_work.as_deref(), Some("subagent working"));
    assert!(workers.ended(&session_id).await.is_none());
    let dispatcher = Dispatcher::acquire(f.layout.clone(), f.home.path().join("config.json"), workers.clone()).await.unwrap();
    let status = dispatcher.status().await.unwrap();
    assert_eq!(status["leads"][0]["sessionId"], session_id);
    assert_eq!(status["projects"][0]["lead"], session_id);
    *wait.lock().unwrap() = Some("bg: build".into());
    assert!(workers.ended(&session_id).await.is_none());
    *wait.lock().unwrap() = Some("quiet: bg: server".into());
    assert!(workers.ended(&session_id).await.is_some());
    let status = dispatcher.status().await.unwrap();
    assert!(status["leads"].as_array().unwrap().is_empty());
    assert!(status["projects"][0]["lead"].is_null());
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let card = hub.list().await.unwrap().into_iter().find(|c| c.session_id == session_id).unwrap();
            if !card.state.is_working() && card.background_work.as_deref() == Some("bg: server") { break; }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }).await.expect("quiet task must not keep the lead working");
    workers.stop(&session_id).await.unwrap();
}
