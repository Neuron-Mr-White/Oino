#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
fn concurrency_cap_is_limited_to_one_through_three() {
    for cap in 1..=3 {
        assert!(Config {
            enabled: true,
            concurrency_cap: cap
        }
        .validate()
        .is_ok());
    }
    for cap in [0, 4, usize::MAX] {
        let error = Config {
            enabled: true,
            concurrency_cap: cap,
        }
        .validate()
        .unwrap_err();
        assert!(error.to_string().contains("between 1 and 3"));
    }
}

#[derive(Default)]
struct Fake {
    live: Mutex<HashSet<String>>,
    launches: AtomicUsize,
    pending: AtomicBool,
    fail_relay: AtomicBool,
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
        Box::pin(async { self.last.lock().unwrap().clone() })
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
    assert!(d.start(&project.slug, &ready).await.is_err());
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
    assert!(d.start(&other.slug, task["id"].as_str().unwrap()).await.is_err());
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
    *fake.last.lock().unwrap() = Some("Need clarification".into()); d.tick().await.unwrap();
    for id in [a, b] { let t = f.task(&id); assert_eq!(t.status, Status::Blocked); assert!(t.run.is_none()); assert!(t.activity.last().unwrap().text.contains("Need clarification")); }
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
async fn crash_mid_launch_blocks_without_second_session() {
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
    assert!(restarted.start(&f.project.slug, &id).await.is_err());
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
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
    let mut names: Vec<String> = session.all_tools().into_iter().map(|t| t.name).collect();
    names.sort();
    for required in ["ask_user", "kanboard", "project_resolver", "superagent.start", "superagent.set_mode", "memory_search", "memory_store"] { assert!(names.iter().any(|n| n == required), "missing {required}: {names:?}"); }
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
async fn global_cap_lowering_and_worker_exit_question() {
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
    assert_eq!(
        d.start(&project.slug, &b).await.unwrap_err().code,
        "E_WORKER_CAP"
    );
    d.configure(Config {
        enabled: false,
        concurrency_cap: 2,
    })
    .await
    .unwrap();
    d.start(&project.slug, &b).await.unwrap();
    d.configure(Config {
        enabled: false,
        concurrency_cap: 1,
    })
    .await
    .unwrap();
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
    workers.stop(&session_id).await.unwrap();
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
    assert!(d.start(&f.project.slug, &id).await.is_err());
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
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
