#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use futures_util::future::BoxFuture;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use unipi_host::superagent::{Config, Dispatcher, INTERRUPTED, Workers};
use unipi_kanboard::{
    Layout, Project,
    board::Board,
    commands::{self, Common},
    model::{Actor, ChainGate, Priority, Status, Task},
};

#[derive(Default)]
struct Fake {
    live: Mutex<HashSet<String>>,
    launches: AtomicUsize,
    pending: AtomicBool,
    fail_relay: AtomicBool,
    relays: Mutex<Vec<String>>,
    last: Mutex<Option<String>>,
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
        concurrency_cap: 4,
    })
    .await
    .unwrap();
    let (a, b) = tokio::join!(d.start(&f.project.slug, &a), d.start(&f.project.slug, &b));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
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
    assert!(
        f.task(&id)
            .activity
            .last()
            .unwrap()
            .text
            .contains(INTERRUPTED)
    );
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
    assert!(
        Dispatcher::acquire(
            f.layout.clone(),
            f.home.path().join("config.json"),
            fake.clone()
        )
        .await
        .is_err()
    );
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
    assert!(
        f.task(&dep)
            .activity
            .last()
            .unwrap()
            .text
            .contains("persist me")
    );
    assert!(d.relay(&f.project.slug, &id, "wrong worker").await.is_err());
}
#[cfg(windows)]
#[tokio::test]
async fn windows_tray_dispatcher() {
    let f = Fixture::new();
    let id = f.add("tray", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    assert!(
        Dispatcher::acquire(
            f.layout.clone(),
            f.home.path().join("config.json"),
            fake.clone()
        )
        .await
        .is_err()
    );
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
    assert!(session.cwd().as_os_str().is_empty());
    assert_eq!(session.active_tool_names().len(), 6);
    let mut names: Vec<String> = session.all_tools().into_iter().map(|t| t.name).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "ask_user",
            "kanboard",
            "project_resolver",
            "superagent.relay",
            "superagent.start",
            "superagent.stop"
        ]
    );
    let shell =
        serde_json::from_value(serde_json::json!({"command":"ls","riskLevel":"low"})).unwrap();
    assert!(
        session
            .check_permission("bash", &shell)
            .await
            .unwrap_err()
            .contains("no workspace")
    );
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
    assert!(resumed.session().cwd().as_os_str().is_empty());
    assert_eq!(resumed.session().all_tools().len(), 6);
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
    assert!(
        f.task(&a)
            .activity
            .last()
            .unwrap()
            .text
            .contains("Which API key should I use?")
    );
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
    let workers = HostWorkers::new(hub.clone());
    let session_id = uuid::Uuid::now_v7().to_string();
    let task = unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &id, &session_id)
        .unwrap();
    workers
        .launch(&f.project, &task, &session_id)
        .await
        .unwrap();
    let runtime = unipi_host::superagent_session::runtime(&session_id).unwrap();
    let session = runtime.session();
    session.wait_for_idle().await;
    assert_eq!(session.session_id(), session_id);
    assert_eq!(
        session.autonomy(),
        oino_core::session::permission::Level::Medium
    );
    session.set_approval_ui(true); // sticky unattended cannot be undone by opening the UI
    assert_eq!(
        session
            .check_permission(
                "bash",
                &serde_json::from_value(
                    serde_json::json!({"command":"git push","riskLevel":"high"})
                )
                .unwrap()
            )
            .await
            .unwrap_err(),
        "needs Autonomy High"
    );
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
    workers.retire(&session_id).await;
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
    assert!(
        Dispatcher::acquire(
            f.layout.clone(),
            f.home.path().join("config.json"),
            fake.clone()
        )
        .await
        .is_err()
    );
    child.kill().unwrap();
    child.wait().unwrap();
    let d = f.dispatcher(fake.clone()).await;
    assert_eq!(f.task(&id).status, Status::Blocked);
    assert!(
        f.task(&id)
            .activity
            .last()
            .unwrap()
            .text
            .contains(INTERRUPTED)
    );
    assert!(d.start(&f.project.slug, &id).await.is_err());
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
}
