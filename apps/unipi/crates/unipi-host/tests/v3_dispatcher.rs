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
    assert!(serde_json::to_value(config)
        .unwrap()
        .get("concurrencyCap")
        .is_none());
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
    agent_dir: Option<std::path::PathBuf>,
    // OIN-80: an errored turn (`stop_reason == Error`) with this message.
    outcome: Mutex<Option<String>>,
    models: Mutex<Vec<String>>,
    continues: Mutex<Vec<String>>,
    notices: AtomicUsize,
    unloads: AtomicUsize,
    files: Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
    services: std::sync::Mutex<Option<std::sync::Arc<oino_core::services::Services>>>,
    // OIN-81: the lead's open ask_user dialogs and the answers delivered into them.
    dialog: Mutex<Vec<unipi_host::superagent_questions::LeadQuestion>>,
    answers: Mutex<Vec<(String, String)>>,
    raw_answers: Mutex<Vec<(String, oino_core::ui::UiAnswer)>>,
    fail_launch: AtomicBool,
    fail_set_model: Mutex<Option<String>>,
    fail_answer_once: AtomicBool,
    fail_notice: AtomicBool,
    notice_log: Mutex<Vec<(String, String, String)>>,
    archived_ids: Mutex<HashSet<String>>,
    dispatcher_weak: std::sync::OnceLock<std::sync::Weak<Dispatcher>>,
    registered_at_launch: Mutex<Vec<(String, bool)>>,
}
impl Workers for Fake {
    fn agent_dir(&self) -> Option<&std::path::Path> {
        self.agent_dir.as_deref()
    }
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
            if let Some(d) = self
                .dispatcher_weak
                .get()
                .and_then(std::sync::Weak::upgrade)
            {
                self.registered_at_launch
                    .lock()
                    .unwrap()
                    .push((id.to_owned(), d.is_lead_session(id)));
            }
            if self.fail_launch.load(Ordering::SeqCst) {
                return Err("launch refused".into());
            }
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
    fn launch_lead<'a>(
        &'a self,
        project: &'a Project,
        tasks: &'a [Task],
        id: &'a str,
        preset: &'a unipi_host::superagent_presets::Preset,
    ) -> BoxFuture<'a, Result<(), String>> {
        self.batches
            .lock()
            .unwrap()
            .push(tasks.iter().map(|t| t.id.clone()).collect());
        self.modes.lock().unwrap().push(preset.mode.clone());
        match tasks.first() {
            Some(task) => self.launch_preset(project, task, id, preset),
            // A wake/probe launch with no tickets: the runtime just reattaches.
            None => Box::pin(async move {
                self.launches.fetch_add(1, Ordering::SeqCst);
                if let Some(d) = self
                    .dispatcher_weak
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                {
                    self.registered_at_launch
                        .lock()
                        .unwrap()
                        .push((id.to_owned(), d.is_lead_session(id)));
                }
                if self.fail_launch.load(Ordering::SeqCst) {
                    return Err("launch refused".into());
                }
                self.live.lock().unwrap().insert(id.into());
                Ok(())
            }),
        }
    }
    fn relay_delivery<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
        delivery: unipi_host::superagent::Delivery,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.relay(id, text).await?;
            self.deliveries
                .lock()
                .unwrap()
                .push((text.to_owned(), delivery));
            Ok(())
        })
    }
    fn set_mode<'a>(&'a self, _: &'a str, mode: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.modes.lock().unwrap().push(mode.into());
            Ok(())
        })
    }
    fn set_model<'a>(&'a self, _: &'a str, model: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(err) = self.fail_set_model.lock().unwrap().clone() {
                return Err(err);
            }
            self.models.lock().unwrap().push(model.into());
            Ok(())
        })
    }
    fn outcome<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            if !self.live.lock().unwrap().contains(id) {
                return None;
            }
            self.outcome.lock().unwrap().clone()
        })
    }
    fn continue_turn<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.continues.lock().unwrap().push(text.to_owned());
            self.relay_delivery(id, text, unipi_host::superagent::Delivery::Queue)
                .await
        })
    }
    fn unload<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.live.lock().unwrap().remove(id);
            self.unloads.fetch_add(1, Ordering::SeqCst);
        })
    }
    fn session_file<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<std::path::PathBuf>> {
        Box::pin(async move { self.files.lock().unwrap().get(id).cloned() })
    }
    fn core_services<'a>(
        &'a self,
        _: &'a Project,
    ) -> BoxFuture<'a, Result<std::sync::Arc<oino_core::services::Services>, String>> {
        Box::pin(async move {
            // Create once; a registry with the faux models the tests reference.
            let existing = self.services.lock().unwrap().clone();
            if let Some(services) = existing {
                return Ok(services);
            }
            let models = oino_models::ModelRegistry::in_memory(std::sync::Arc::new(
                oino_models::MapEnv::default(),
            ));
            models.set_runtime_api_key("faux", "test-key");
            for id in ["example", "fallback", "other"] {
                models
                    .register_virtual_model(
                        "test",
                        oino_models::VirtualModelDefinition {
                            provider: "faux".into(),
                            id: id.into(),
                            name: id.into(),
                            thinking_levels: None,
                            context_window: None,
                            max_tokens: None,
                            input: None,
                        },
                    )
                    .unwrap();
            }
            let services =
                oino_core::services::Services::create(oino_core::services::ServicesOptions {
                    cwd: std::env::temp_dir(),
                    agent_dir: std::env::temp_dir().join("oino-v3-dispatcher-agent"),
                    docs_root: std::env::temp_dir().join("oino-v3-dispatcher-docs"),
                    mode: oino_core::extensions::api::ExtensionMode::Rpc,
                    has_ui: false,
                    flags: oino_core::services::ResourceFlags {
                        no_context_files: true,
                        ..oino_core::services::ResourceFlags::default()
                    },
                    extension_flag_values: indexmap::IndexMap::default(),
                    loader: std::sync::Arc::new(oino_core::services::StaticLoader(Vec::new())),
                    approve: Some(false),
                    models: Some(models),
                    settings: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            *self.services.lock().unwrap() = Some(services.clone());
            Ok(services)
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
        Box::pin(async {
            if self.background.load(Ordering::SeqCst) {
                None
            } else {
                self.last.lock().unwrap().clone()
            }
        })
    }
    fn retire<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {
            self.retirements.fetch_add(1, Ordering::SeqCst);
        })
    }
    fn questions<'a>(
        &'a self,
        _: &'a str,
    ) -> BoxFuture<'a, Vec<unipi_host::superagent_questions::LeadQuestion>> {
        Box::pin(async { self.dialog.lock().unwrap().clone() })
    }
    fn answer<'a>(
        &'a self,
        _: &'a str,
        request_id: &'a str,
        answer: oino_core::ui::UiAnswer,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.fail_answer_once.swap(false, Ordering::SeqCst) {
                return Err("dialog respond failed".into());
            }
            let mut dialogs = self.dialog.lock().unwrap();
            let Some(index) = dialogs.iter().position(|q| q.request_id == request_id) else {
                return Err("The question is no longer open".into());
            };
            let text = match &answer {
                oino_core::ui::UiAnswer::Value(value) => {
                    if value["answers"].as_array().is_some_and(Vec::is_empty) {
                        "<skipped>".to_owned()
                    } else if let Some(sel) = value["answers"][0]["selected"][0].as_str() {
                        sel.to_owned()
                    } else {
                        value["answers"][0]["other"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned()
                    }
                }
                oino_core::ui::UiAnswer::Cancelled => "<cancelled>".to_owned(),
                oino_core::ui::UiAnswer::Confirmed(b) => format!("confirmed:{b}"),
            };
            dialogs.remove(index);
            self.raw_answers
                .lock()
                .unwrap()
                .push((request_id.to_owned(), answer));
            self.answers
                .lock()
                .unwrap()
                .push((request_id.to_owned(), text));
            Ok(())
        })
    }
    fn lead_notice<'a>(
        &'a self,
        id: &'a str,
        title: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            if self.fail_notice.load(Ordering::SeqCst) {
                return false;
            }
            self.notices.fetch_add(1, Ordering::SeqCst);
            self.notice_log.lock().unwrap().push((
                id.to_owned(),
                title.to_owned(),
                message.to_owned(),
            ));
            true
        })
    }
    fn is_session_archived<'a>(
        &'a self,
        id: &'a str,
        session_file: Option<&'a std::path::Path>,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            if self.archived_ids.lock().unwrap().contains(id) {
                return true;
            }
            let Some(dir) = self.agent_dir.as_deref() else {
                return false;
            };
            unipi_host::superagent_workers::session_in_trash(&dir.join("trash"), id, session_file)
        })
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
    assert_eq!(
        d.start("missing", &task).await.unwrap_err().code,
        "E_PROJECT_NOT_REGISTERED"
    );
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
    let mut archived =
        Project::create(&f.layout, archived_root.path(), Some("archived"), None).unwrap();
    archived.archived = true;
    archived.save(&f.layout).unwrap();
    let roots: Vec<_> = (0..7).map(|_| tempfile::tempdir().unwrap()).collect();
    let group = |path: &std::path::Path| ProjectGroup {
        cwd: path.to_string_lossy().into(),
        name: "workspace".into(),
        cards: vec![],
    };
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
    assert_eq!(
        d.blockers("missing").await.unwrap()[0].code,
        "not_registered"
    );
    assert_eq!(
        d.enable("enable_project_auto", Some(&f.project.slug), None)
            .await
            .unwrap()["ok"],
        true
    );
    assert_eq!(d.status().await.unwrap()["projects"][0]["autoStart"], true);
    assert_eq!(
        d.enable("enable_auto_start", None, None).await.unwrap()["projectId"],
        serde_json::Value::Null
    );
    assert_eq!(d.status().await.unwrap()["enabled"], true);
    let mut archived = f.project.clone();
    archived.archived = true;
    archived.save(&f.layout).unwrap();
    d.enable("unarchive", Some(&archived.slug), None)
        .await
        .unwrap();
    assert!(!Project::load(&f.layout, &archived.slug).unwrap().archived);
    let root = tempfile::tempdir().unwrap();
    let result = d
        .enable("register", None, Some(root.path().to_str().unwrap()))
        .await
        .unwrap();
    let created = Project::load(&f.layout, result["projectId"].as_str().unwrap()).unwrap();
    assert_eq!(created.root, root.path().canonicalize().unwrap());
    assert_eq!(
        d.enable("register", None, Some("relative"))
            .await
            .unwrap_err()
            .code,
        "E_SCHEMA"
    );
    assert_eq!(
        d.enable("unknown", None, None).await.unwrap_err().code,
        "E_SCHEMA"
    );
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
            ..unipi_host::superagent_presets::Overrides::default()
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
        ..Config::default()
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
    for (id, status) in [
        (&archived_task, Status::Archived),
        (&review, Status::InReview),
    ] {
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
    assert_eq!(
        d.start(&project.slug, &ready).await.unwrap_err().code,
        "E_PROJECT_ARCHIVED"
    );
    d.configure_project(&project.slug, true).await.unwrap();
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
        ..Config::default()
    })
    .await
    .unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[allow(
    clippy::many_single_char_names,
    reason = "Short fixture and stream endpoint names follow the protocol scenario notation."
)]
async fn multiple_tasks_feed_one_lead_and_route_delivery() {
    use unipi_host::superagent::Delivery;
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let b = f.add("second", &[]);
    let c = f.add("third", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let id = d
        .start_tasks(
            &f.project.slug,
            &[a.clone(), b.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
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
    d.relay_to_lead(
        &f.project.slug,
        "complement",
        std::slice::from_ref(&b),
        Delivery::Steer,
        true,
    )
    .await
    .unwrap();
    d.relay_to_lead(
        &f.project.slug,
        "dependent",
        std::slice::from_ref(&c),
        Delivery::Queue,
        false,
    )
    .await
    .unwrap();
    {
        let deliveries = fake.deliveries.lock().unwrap();
        assert_eq!(deliveries[1].1, Delivery::Steer);
        assert!(deliveries[1]
            .0
            .starts_with("URGENT: schedule this first / add it as the first graph item"));
        assert_eq!(deliveries[2], ("dependent".into(), Delivery::Queue));
    }
    assert!(f
        .task(&b)
        .activity
        .iter()
        .any(|a| a.text.contains("[superagent]") && a.text.contains("complement")));
    let root = tempfile::tempdir().unwrap();
    let other = Project::create(&f.layout, root.path(), Some("other"), Some("OT")).unwrap();
    let task = commands::add(
        &f.layout,
        other.clone(),
        &Common::new(Actor::User, ChainGate::InReview),
        "other",
        None,
        Some(Status::Todo),
        Priority::Medium,
        &[],
        &[],
        &[],
        false,
    )
    .unwrap();
    assert_ne!(
        d.start(&other.slug, task["id"].as_str().unwrap())
            .await
            .unwrap(),
        id
    );
}

#[tokio::test]
async fn auto_start_batches_all_ready_tickets() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let b = f.add("second", &[]);
    let _blocked = f.add("wait", std::slice::from_ref(&a));
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
        ..Config::default()
    })
    .await
    .unwrap();
    d.configure_project(&f.project.slug, true).await.unwrap();
    d.tick().await.unwrap();
    let batches = fake.batches.lock().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 2);
    assert!(batches[0].contains(&a));
    assert!(batches[0].contains(&b));
}

#[tokio::test]
async fn preset_mode_and_switch_reach_lead_and_status() {
    let f = Fixture::new();
    let task = f.add("mode", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let id = d
        .start_tasks(
            &f.project.slug,
            &[task],
            Some("feature"),
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    assert_eq!(fake.modes.lock().unwrap().as_slice(), ["graph"]);
    assert_eq!(d.status().await.unwrap()["leads"][0]["mode"], "graph");
    assert_eq!(
        d.set_mode(&f.project.slug, "swarm").await.unwrap(),
        serde_json::json!({"sessionId":id,"mode":"swarm"})
    );
    assert_eq!(fake.modes.lock().unwrap().as_slice(), ["graph", "swarm"]);
    assert_eq!(d.status().await.unwrap()["leads"][0]["mode"], "swarm");
    assert!(d.set_mode(&f.project.slug, "invalid").await.is_err());
}

#[tokio::test]
#[allow(
    clippy::many_single_char_names,
    reason = "Short fixture and stream endpoint names follow the protocol scenario notation."
)]
async fn completed_lead_retires_and_new_work_starts_fresh() {
    // OIN-80 rewrite: completion goes Idle, and new work wakes the SAME chat.
    let f = Fixture::new();
    let a = f.add("review", &[]);
    let b = f.add("blocked", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d
        .start_tasks(
            &f.project.slug,
            &[a.clone(), b.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &a,
        &lead,
        "done",
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &b,
        &lead,
        Some("Which option?"),
    )
    .unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    assert_eq!(f.task(&a).status, Status::InReview);
    assert_eq!(f.task(&b).status, Status::Blocked);
    assert_eq!(
        f.task(&b).run.unwrap().session,
        lead,
        "the blocked ticket keeps its claim"
    );
    // The needsYou row for a blocked ticket that KEEPS its claim is OIN-81 (B2).
    *fake.outcome.lock().unwrap() = None;
    let c = f.add("fresh", &[]);
    assert_eq!(
        d.start(&f.project.slug, &c).await.unwrap(),
        lead,
        "new work wakes the same session"
    );
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[allow(
    clippy::many_single_char_names,
    reason = "Short fixture and stream endpoint names follow the protocol scenario notation."
)]
async fn ended_lead_blocks_every_in_progress_ticket_with_last_message() {
    // OIN-80 rewrite: a plain end with tickets held stalls (NoProgress health)
    // after the nudge budget; the tickets keep their claims and stay in_progress.
    let f = Fixture::new();
    let a = f.add("a", &[]);
    let b = f.add("b", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start_tasks(
        &f.project.slug,
        &[a.clone(), b.clone()],
        None,
        unipi_host::superagent_presets::Overrides::default(),
    )
    .await
    .unwrap();
    *fake.outcome.lock().unwrap() = Some("Need clarification".into());
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(status["leads"][0]["health"]["kind"], "no_progress");
    assert_eq!(
        status["needsYou"].as_array().unwrap().len(),
        0,
        "a stall is health, not a blocked ticket"
    );
    for id in [a, b] {
        let t = f.task(&id);
        assert_eq!(t.status, Status::InProgress);
        assert_eq!(t.run.unwrap().session, status["leads"][0]["sessionId"]);
    }
}

#[tokio::test]
async fn idle_lead_with_background_graph_is_not_retired() {
    let f = Fixture::new();
    let task = f.add("graph", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &task).await.unwrap();
    fake.background.store(true, Ordering::SeqCst);
    d.tick().await.unwrap();
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(fake.retirements.load(Ordering::SeqCst), 0);
    assert!(fake.continues.lock().unwrap().is_empty());
}

#[tokio::test]
async fn idle_lead_gets_one_continuation_then_is_blocked() {
    // OIN-80 rewrite: the nudge budget is consumed across reacquisition, then
    // the lead stalls — the ticket stays in_progress with its claim.
    let f = Fixture::new();
    let task = f.add("idle", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(fake.continues.lock().unwrap().len(), 1);
    assert!(fake.continues.lock().unwrap()[0].contains(&task));
    // Reacquisition preserves the consumed budget (the second nudge still fits);
    // the resume itself adds the RESTART_TEXT continue.
    drop(d);
    let d = f.dispatcher(fake.clone()).await;
    assert_eq!(
        fake.continues.lock().unwrap().len(),
        2,
        "restart resume continues once"
    );
    d.tick().await.unwrap();
    assert_eq!(
        fake.continues.lock().unwrap().len(),
        3,
        "the second nudge still fits"
    );
    // The budget is out: the next tick stalls the lead, no third nudge.
    d.tick().await.unwrap();
    assert_eq!(fake.continues.lock().unwrap().len(), 3);
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(
        f.task(&task).run.unwrap().session,
        status["leads"][0]["sessionId"]
    );
}

#[tokio::test]
#[allow(
    clippy::many_single_char_names,
    reason = "Short fixture and stream endpoint names follow the protocol scenario notation."
)]
async fn graph_continuation_names_remaining_tickets_and_progress_replenishes_budget() {
    let f = Fixture::new();
    let a = f.add("a", &[]);
    let b = f.add("b", &[]);
    let c = f.add("c", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d
        .start_tasks(
            &f.project.slug,
            &[a.clone(), b.clone(), c.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    d.set_mode(&f.project.slug, "graph").await.unwrap();
    assert_eq!(fake.modes.lock().unwrap().last().unwrap(), "graph");
    d.tick().await.unwrap(); // record initial claims while working
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &a,
        &lead,
        "done",
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    let text = fake.continues.lock().unwrap()[0].clone();
    assert!(!text.contains(&a));
    assert!(text.contains(&b));
    assert!(text.contains(&c));
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &b,
        &lead,
        "done",
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    d.tick().await.unwrap();
    assert_eq!(fake.continues.lock().unwrap().len(), 2);
    assert!(fake.continues.lock().unwrap()[1].contains(&c));
    // Finishing `a` replenished the budget, so the lead earns one more nudge.
    d.tick().await.unwrap();
    assert_eq!(fake.continues.lock().unwrap().len(), 3);
    d.tick().await.unwrap();
    // OIN-80: the budget is out — the lead stalls, the ticket stays in_progress.
    assert_eq!(f.task(&c).status, Status::InProgress);
    assert_eq!(d.status().await.unwrap()["leads"][0]["state"], "stalled");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
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
        always_available: true,
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
        serde_json::json!({"kind":"approval","projectId":f.project.slug,"taskId":id,"status":"blocked","workerSessionId":session,"requestId":"request","sessionId":"child","tool":"bash","command":"git push","summary":"push","risk":"high","ceiling":"medium","reason":"needs Autonomy High"})
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
    // Owner decision: a stop retires the lead; BOTH in_progress and blocked
    // tickets move to backlog with `run` cleared and `Released incomplete:`.
    assert_eq!(f.task(&id).status, Status::Backlog);
    assert!(f.task(&id).run.is_none());
    assert!(f
        .task(&id)
        .activity
        .last()
        .unwrap()
        .text
        .starts_with(unipi_kanboard::dispatcher::RELEASED_INCOMPLETE_PREFIX));
    assert_eq!(
        d.status().await.unwrap()["leads"]
            .as_array()
            .unwrap()
            .as_slice(),
        [] as [serde_json::Value; 0]
    );

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
    drop(d);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _restarted = f.dispatcher(fake.clone()).await;
    // OIN-80: the new lead has a Working record, so the restart RESUMES it on
    // the same session id (a recorded claim is never parked with INTERRUPTED).
    let task = f.task(&another);
    assert_eq!(task.status, Status::Blocked);
    assert_eq!(
        task.run.as_ref().map(|r| r.session.as_str()),
        Some(session.as_str())
    );
    assert_ne!(task.activity.last().unwrap().text, INTERRUPTED);
    assert!(fake
        .continues
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains("host restarted"));
    // The first lead's stop retired it; its blocked ticket moved to backlog with
    // `run = None` and `Released incomplete:` preserving the open question — the
    // stop never resumes a retired lead.
    let first = f.task(&id);
    assert_eq!(first.status, Status::Backlog);
    assert!(first.run.is_none());
    assert!(first
        .activity
        .last()
        .unwrap()
        .text
        .contains("git push risk high"));
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
        ..Config::default()
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
        ..Config::default()
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
    assert_ne!(reserved, "");
    launch.abort();
    let _ = launch.await;
    drop(d);
    // A concurrent subprocess test may briefly inherit an fd between fork and exec.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // OIN-80: the start persisted a Working record BEFORE the launch, so the
    // crash resumes the SAME session id (claim kept) instead of minting one.
    // The resume launch blocks on `pending`; drive it from a task.
    let fake2 = Arc::new(Fake::default());
    let acquired = {
        let layout = f.layout.clone();
        let config = f.home.path().join("config.json");
        let fake2 = fake2.clone();
        tokio::spawn(async move { Dispatcher::acquire(layout, config, fake2).await })
    };
    while fake2.launches.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let restarted = acquired.await.unwrap().unwrap();
    assert_eq!(
        f.task(&id).status,
        Status::InProgress,
        "the claim survives the crash"
    );
    assert_eq!(f.task(&id).run.unwrap().session, reserved);
    let status = restarted.status().await.unwrap();
    assert_eq!(
        status["leads"][0]["sessionId"], reserved,
        "the same session id resumes"
    );
    assert_eq!(status["leads"][0]["state"], "working");
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn superagent_has_only_cross_project_tools_and_no_cwd() {
    use oino_core::{
        extensions::api::ExtensionMode,
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
    };
    use unipi_host::sessions::{HostHubOptions, SessionHub};
    let dir = tempfile::tempdir().unwrap();
    let memory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../extensions/dist/unipi.memory");
    assert!(memory.join("component.wasm").is_file());
    let extensions = dir.path().join("agent/extensions");
    std::fs::create_dir_all(&extensions).unwrap();
    std::os::unix::fs::symlink(&memory, extensions.join("unipi.memory")).unwrap();
    // The user's coding SYSTEM.md / APPEND_SYSTEM.md must not reach the Superagent.
    std::fs::write(dir.path().join("agent/SYSTEM.md"), "USER CODING PROMPT").unwrap();
    std::fs::write(dir.path().join("agent/APPEND_SYSTEM.md"), "USER APPENDIX").unwrap();
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
            ..oino_core::services::ResourceFlags::default()
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
    let prompt = session.system_prompt();
    assert!(prompt.starts_with("# Superagent"), "{prompt}");
    assert!(
        !prompt.contains("USER CODING PROMPT") && !prompt.contains("USER APPENDIX"),
        "{prompt}"
    );
    assert_eq!(session.cwd(), dir.path().join("agent/superagent"));
    assert!(session
        .active_tool_names()
        .contains(&"memory_search".to_owned()));
    assert!(session
        .active_tool_names()
        .contains(&"memory_store".to_owned()));
    let tools = session.all_tools();
    oin31::audit_and_wire(&tools);
    for name in [
        "superagent.start",
        "superagent_start",
        "superagent.set_mode",
        "superagent_set_mode",
    ] {
        let args = serde_json::from_value(
            serde_json::json!({"projectId":"test", "taskIds":["T-1"], "mode":"goal"}),
        )
        .unwrap();
        assert!(
            session.check_permission(name, &args).await.is_ok(),
            "permission denied: {name}"
        );
    }
    let resolver = tools.iter().find(|t| t.name == "project_resolver").unwrap();
    let call: oino_ai::ToolCall = serde_json::from_value(serde_json::json!({"id":"call_1","name":"project_resolver","arguments":{"reason":"find project"}})).unwrap();
    assert!(oino_agent::validate_tool_arguments(&resolver.parameters, &call).is_ok());
    let empty: oino_ai::ToolCall = serde_json::from_value(
        serde_json::json!({"id":"call_2","name":"project_resolver","arguments":{}}),
    )
    .unwrap();
    assert!(oino_agent::validate_tool_arguments(&resolver.parameters, &empty).is_ok());
    let mut names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    names.sort();
    for required in [
        "ask_user",
        "kanboard",
        "project_resolver",
        "superagent_start",
        "superagent_set_mode",
        "memory_search",
        "memory_store",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "missing {required}: {names:?}"
        );
    }
    for forbidden in ["read", "write", "edit", "bash", "grep", "find", "ls"] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "unexpected {forbidden}"
        );
    }
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
    assert!(resumed
        .session()
        .active_tool_names()
        .contains(&"memory_search".to_owned()));
    resumed.dispose().await;
}

#[tokio::test]
async fn multiple_projects_dispatch_without_a_global_limit() {
    let f = Fixture::new();
    let first = f.add("oino work", &[]);
    let roots: Vec<_> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let projects: Vec<_> = ["demoeditor-migration", "example-api", "manual"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            Project::create(
                &f.layout,
                roots[i].path(),
                Some(name),
                Some(&format!("P{i}")),
            )
            .unwrap()
        })
        .collect();
    let add = |project: &Project, title: &str| {
        commands::add(
            &f.layout,
            project.clone(),
            &Common::new(Actor::User, ChainGate::InReview),
            title,
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
            .to_owned()
    };
    let zm = add(&projects[0], "zm work");
    for i in 0..8 {
        add(&projects[1], &format!("cloud {i}"));
    }
    let manual = add(&projects[2], "manual work");
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &first).await.unwrap();
    d.start(&projects[0].slug, &zm).await.unwrap();
    d.configure(Config {
        enabled: true,
        concurrency_cap: 1,
        ..Config::default()
    })
    .await
    .unwrap();
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
    *fake.outcome.lock().unwrap() = Some("Which API key should I use?".into());
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    // OIN-80: an ended turn with tickets held stalls both leads; the tickets
    // keep their claims and stay in_progress (the question stays on the lead).
    assert_eq!(f.task(&a).status, Status::InProgress);
    assert_eq!(
        Board::open(&f.layout, project)
            .unwrap()
            .get(&b)
            .unwrap()
            .status,
        Status::InProgress
    );
    let status = d.status().await.unwrap();
    let leads = status["leads"].as_array().unwrap();
    assert_eq!(leads.len(), 2);
    assert!(leads.iter().all(|lead| lead["state"] == "stalled"));
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
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
            ..oino_core::services::ResourceFlags::default()
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
        .resolve(
            Some("research"),
            unipi_host::superagent_presets::Overrides::default(),
        )
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
            if matches!(
                event,
                oino_core::events::ClientEvent::Oino(
                    oino_core::events::OinoEvent::SessionEvicted { .. }
                )
            ) {
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
    // OIN-80: the acquire migrated the claim into a Working record, so the
    // restart resumes it on the same session id instead of parking it.
    let task = f.task(&id);
    assert_eq!(task.status, Status::Blocked);
    assert_eq!(
        task.run.as_ref().map(|r| r.session.as_str()),
        Some(session_id.as_str())
    );
    assert_ne!(task.activity.last().unwrap().text, INTERRUPTED);
    drop(restarted);
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
    // OIN-80: the start persisted a Working record before the launch, so the
    // crash resumes the SAME session id instead of parking the claim.
    let task = f.task(&id);
    assert_eq!(task.status, Status::InProgress);
    assert_ne!(task.activity.last().unwrap().text, INTERRUPTED);
    // The resume re-attached once; more work queues to the SAME lead (no new id).
    let record = d
        .leads()
        .lock()
        .unwrap()
        .leads
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(record.session_id, task.run.as_ref().unwrap().session);
    assert!(
        record.resume_sent_at.is_some(),
        "the resume re-attached once"
    );
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn status_reports_only_live_leads() {
    // OIN-80 rewrite: records keep the lead listed with its state even when the
    // runtime is gone (stalled/unloaded leads are visible).
    let f = Fixture::new();
    let task = f.add("lead", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let id = d.start(&f.project.slug, &task).await.unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["projects"][0]["lead"], id);
    assert_eq!(status["leads"].as_array().unwrap().len(), 1);
    assert_eq!(status["leads"][0]["state"], "working");
    assert!(
        status["leads"][0]["lastActiveAt"].is_u64(),
        "epoch-ms for every state (finding 10; the fleet merge refreshes live rows)"
    );
    assert!(status["leads"][0]["health"].is_null());
    *fake.outcome.lock().unwrap() = Some("provider error: 429 quota".into());
    d.tick().await.unwrap();
    fake.live.lock().unwrap().remove(&id);
    let status = d.status().await.unwrap();
    // Without fallback configured, the first error already stalls the lead —
    // still listed, with health (OIN-80's point: it must not vanish).
    assert_eq!(status["leads"].as_array().unwrap().len(), 1);
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(status["leads"][0]["health"]["error"], "429 quota");
    assert!(status["leads"][0]["idleReason"].is_null());
}

#[tokio::test]
async fn status_counts_ready_tickets_using_dispatch_rule() {
    let f = Fixture::new();
    let first = f.add("ready", &[]);
    f.add("blocked dependency", std::slice::from_ref(&first));
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
                ..unipi_host::superagent_presets::Overrides::default()
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
                ..unipi_host::superagent_presets::Overrides::default()
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
        ..Config::default()
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
                ..unipi_host::superagent_presets::Overrides::default()
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
                ..unipi_host::superagent_presets::Overrides::default()
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
                ..unipi_host::superagent_presets::Overrides::default()
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
        ..Config::default()
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
            ..unipi_host::superagent_presets::Overrides::default()
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn unattached_lead_tracks_waking_but_not_quiet_work() {
    use oino_core::{
        extensions::api::{ExtensionInstance, ExtensionMode, Registration},
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
        testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse},
    };
    use unipi_host::{
        sessions::{HostHubOptions, SessionHub},
        superagent::HostWorkers,
    };
    let f = Fixture::new();
    let id = f.add("background lead", &[]);
    let wait = Arc::new(Mutex::new(Some("subagent working".to_owned())));
    let source = {
        let wait = Arc::clone(&wait);
        FakeExtension::new("waiter", Registration::default())
            .on_wait(move |_| wait.lock().unwrap().clone())
    };
    let provider = FauxProvider::new(vec![FauxResponse::Text("yielded".into())]);
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
            ..oino_core::services::ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(vec![
            Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>,
            Arc::new(source),
        ])),
        models: Some(models),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    let workers = Arc::new(HostWorkers::new(hub.clone(), f.layout.clone()));
    let session_id = uuid::Uuid::now_v7().to_string();
    let task = unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &id, &session_id)
        .unwrap();
    let mut preset = unipi_host::superagent_presets::Store::default()
        .resolve(
            Some("research"),
            unipi_host::superagent_presets::Overrides::default(),
        )
        .unwrap();
    preset.model = Some("faux/faux-1".into());
    workers
        .launch_preset(&f.project, &task, &session_id, &preset)
        .await
        .unwrap();
    let session = unipi_host::superagent_session::runtime(&session_id)
        .unwrap()
        .session();
    session.wait_for_idle().await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let card = hub
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.session_id == session_id)
        .unwrap();
    assert!(card.state.is_working(), "{card:?}");
    assert_eq!(card.pending_work.as_deref(), Some("subagent working"));
    assert!(workers.ended(&session_id).await.is_none());
    let dispatcher = Dispatcher::acquire(
        f.layout.clone(),
        f.home.path().join("config.json"),
        workers.clone(),
    )
    .await
    .unwrap();
    // OIN-80: acquire migrated the claim into a Working record and resumed it —
    // the restart continue starts one more turn. Let it settle before probing.
    session.wait_for_idle().await;
    let status = dispatcher.status().await.unwrap();
    assert_eq!(status["leads"][0]["sessionId"], session_id);
    assert_eq!(status["projects"][0]["lead"], session_id);
    *wait.lock().unwrap() = Some("bg: build".into());
    assert!(workers.ended(&session_id).await.is_none());
    *wait.lock().unwrap() = Some("quiet: bg: server".into());
    assert!(workers.ended(&session_id).await.is_some());
    // OIN-80: the record keeps the lead listed (Working) even when quiet; the
    // legacy `projects[].lead` only tracks a live, non-ended worker.
    let status = dispatcher.status().await.unwrap();
    assert_eq!(status["leads"].as_array().unwrap().len(), 1);
    assert!(status["projects"][0]["lead"].is_null());
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let card = hub
                .list()
                .await
                .unwrap()
                .into_iter()
                .find(|c| c.session_id == session_id)
                .unwrap();
            if !card.state.is_working() && card.background_work.as_deref() == Some("bg: server") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("quiet task must not keep the lead working");
    workers.stop(&session_id).await.unwrap();
}

#[tokio::test]
async fn paused_graph_policy_defaults_to_report_and_status_attaches_parked_graph() {
    use unipi_host::superagent::PausedGraphPolicy;
    let f = Fixture::new();
    let task = f.add("follow-up ticket", &[]);
    let agent_dir = f.home.path().join("agent");
    let ws_dir =
        agent_dir
            .join("state/unipi.long-horizon/workspace")
            .join(oino_wasm_host::workspace_id(
                &f.project.root.canonicalize().unwrap(),
            ));
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("state.json"),
        serde_json::to_vec(&serde_json::json!({
            "parked": {
                "ownerId": "owner-1",
                "kind": "graph",
                "label": "Auth refactor graph",
                "session": "lead-prev-1",
                "status": "parked",
                "reason": "paused(superseded_by:none)",
                "revision": 2,
                "lease": { "ownerId": "owner-1", "generation": 1 },
                "updatedAt": "2026-11-09T00:00:00Z"
            },
            "history": []
        }))
        .unwrap(),
    )
    .unwrap();

    let fake = Arc::new(Fake {
        agent_dir: Some(agent_dir),
        ..Fake::default()
    });
    let d = f.dispatcher(fake.clone()).await;

    assert_eq!(
        Config::default().paused_graph_policy,
        PausedGraphPolicy::Report
    );
    let initial = d.status().await.unwrap();
    assert_eq!(initial["pausedGraphPolicy"], "report");

    let saved = d
        .configure(Config {
            enabled: true,
            paused_graph_policy: PausedGraphPolicy::Reuse,
            ..Config::default()
        })
        .await
        .unwrap();
    assert_eq!(saved.paused_graph_policy, PausedGraphPolicy::Reuse);
    assert_eq!(d.status().await.unwrap()["pausedGraphPolicy"], "reuse");

    let new_lead = d.start(&f.project.slug, &task).await.unwrap();
    assert_ne!(new_lead, "lead-prev-1");

    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["sessionId"], new_lead);
    assert_eq!(status["leads"][0]["mode"], "none");
    assert_eq!(
        status["leads"][0]["pausedGraph"],
        serde_json::json!({
            "sessionId": "lead-prev-1",
            "title": "Auth refactor graph"
        })
    );

    // Once resumed (moved to active) or owned by the running lead itself, pausedGraph clears.
    std::fs::write(
        ws_dir.join("state.json"),
        serde_json::to_vec(&serde_json::json!({
            "active": {
                "ownerId": "owner-1",
                "kind": "graph",
                "label": "Auth refactor graph",
                "session": "lead-prev-1",
                "status": "active"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let resumed_status = d.status().await.unwrap();
    assert!(resumed_status["leads"][0].get("pausedGraph").is_none());

    drop(d);
    let reopened = f.dispatcher(fake).await;
    assert_eq!(
        reopened.status().await.unwrap()["pausedGraphPolicy"],
        "reuse"
    );
}

// ===================== OIN-80 tests (spec §4) =====================

/// The owner's #1 complaint: repeated provider errors through many ticks must
/// keep `launches == 1` and one session id — a 429 never mints a chat.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn errored_turn_continues_on_same_session() {
    let f = Fixture::new();
    let task = f.add("quota", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        lead: unipi_host::superagent_leads::LeadSettings {
            retries: 3,
            fallback_model: Some("faux/fallback".into()),
            ..unipi_host::superagent_leads::LeadSettings::default()
        },
        ..Config::default()
    })
    .await
    .unwrap();
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    // A preset model so the probe has a primary to return to.
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .preset
        .model = Some("faux/example".into());
    let now = unipi_host::superagent_leads::now_ms();
    // Repeated 429s through many ticks.
    *fake.outcome.lock().unwrap() =
        Some("provider error: 429 quota exhausted, reset after 79h".into());
    for _ in 0..5 {
        d.tick_at(now + 1_000).await.unwrap();
    }
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        1,
        "an error never mints a chat"
    );
    assert_eq!(
        fake.models.lock().unwrap().as_slice(),
        ["faux/fallback"],
        "first error switches to the fallback"
    );
    let status = d.status().await.unwrap();
    assert_eq!(
        status["leads"][0]["sessionId"], lead,
        "one session id throughout"
    );
    // The second error cannot fall back again → Stalled, health attempts 2.
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(status["leads"][0]["health"]["attempts"], 2);
    assert_eq!(status["leads"][0]["health"]["kind"], "model");
    assert_eq!(status["needsYou"].as_array().unwrap().len(), 0);
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "one stall notice per episode"
    );
    {
        let leads = d.leads().lock().unwrap();
        assert_eq!(leads.leads.len(), 1);
        assert!(
            leads
                .leads
                .values()
                .next()
                .unwrap()
                .health
                .as_ref()
                .unwrap()
                .notified
        );
    }
    // More ticks keep everything stable: same id, no launch, no new notification.
    for _ in 0..3 {
        d.tick_at(now + 2_000).await.unwrap();
    }
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "repeated ticks never re-notify"
    );
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    assert_eq!(d.status().await.unwrap()["leads"][0]["sessionId"], lead);
    // The ticket keeps its claim.
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(f.task(&task).run.unwrap().session, lead);
    // The reset hint (79h) schedules the probe — +30 min does nothing yet
    // (finding 4: a relative hint is added to now; the old code probed after
    // one minute).
    d.tick_at(now + unipi_host::superagent_leads::STALL_REPROBE_MS + 1_000)
        .await
        .unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    assert_eq!(
        fake.models.lock().unwrap().as_slice(),
        ["faux/fallback"],
        "the 79h hint defers the probe"
    );
    // At the hint time the probe wakes on the record's preset model and continues.
    d.tick_at(now + 79 * 3_600_000 + 60_000).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fake.models.lock().unwrap().last().unwrap(),
        "faux/example",
        "the probe returns to the record's preset model"
    );
    assert!(fake
        .continues
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains("provider error was"));
    assert_eq!(d.status().await.unwrap()["leads"][0]["state"], "working");
    // Success clears the health (Done with no error text).
    *fake.outcome.lock().unwrap() = Some(String::new());
    d.tick_at(now + unipi_host::superagent_leads::STALL_REPROBE_MS + 2_000)
        .await
        .unwrap();
    let status = d.status().await.unwrap();
    assert!(
        status["leads"][0]["health"].is_null(),
        "{:?}",
        status["leads"][0]["health"]
    );
}

/// Restart (Q3): drop and re-acquire with empty `live` → same session resumed.
#[tokio::test]
async fn restart_reattaches_and_resumes() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    // The launch captured a session file (the restart resume reattaches to it).
    *fake.files.lock().unwrap() = [(lead.clone(), std::path::PathBuf::from("/tmp/lead.jsonl"))]
        .into_iter()
        .collect();
    drop(d);
    let fake2 = Arc::new(Fake::default());
    let d = f.dispatcher(fake2.clone()).await;
    assert_eq!(
        f.task(&a).run.unwrap().session,
        lead,
        "the claim survives the restart without INTERRUPTED"
    );
    let t = f.task(&a);
    assert_ne!(
        t.activity.last().unwrap().text,
        unipi_host::superagent::INTERRUPTED
    );
    assert_eq!(
        fake2.batches.lock().unwrap().len(),
        1,
        "resume re-launched once"
    );
    assert_eq!(fake2.continues.lock().unwrap().len(), 1);
    assert!(fake2.continues.lock().unwrap()[0].contains("host restarted"));
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].session_id,
        lead
    );
    assert!(d.leads().lock().unwrap().leads[&f.project.slug]
        .resume_sent_at
        .is_some());
}

/// Warm idle (Q9/c): Done → Idle; past warmMinutes → Unloaded; new work wakes
/// the SAME chat with a brief for the new ticket.
#[tokio::test]
async fn new_work_wakes_unloaded_lead_with_brief() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &a,
        &lead,
        "done",
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    assert_eq!(d.status().await.unwrap()["leads"][0]["state"], "idle");
    // Past the warm window the lead unloads; the record and file stay.
    d.tick_at(now + 31 * 60_000).await.unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "unloaded");
    assert_eq!(status["leads"][0]["idleReason"], "unloaded");
    assert_eq!(fake.unloads.load(Ordering::SeqCst), 1);
    // New work reopens the same chat (initial launch + the wake = 2 launches).
    *fake.outcome.lock().unwrap() = None;
    let b = f.add("second", &[]);
    let id = d.start(&f.project.slug, &b).await.unwrap();
    assert_eq!(id, lead, "the unloaded lead's chat is reused");
    assert_eq!(fake.launches.load(Ordering::SeqCst), 2);
    assert_eq!(*fake.batches.lock().unwrap().last().unwrap(), vec![b]);
}

/// (d) the resolved preset survives the restart.
#[tokio::test]
async fn preset_survives_restart() {
    use unipi_host::superagent_presets::Overrides;
    let f = Fixture::new();
    let task = f.add("feature work", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d
        .start_with_preset(
            &f.project.slug,
            &task,
            Some("feature"),
            Overrides::default(),
        )
        .await
        .unwrap();
    drop(d);
    let fake2 = Arc::new(Fake::default());
    let d = f.dispatcher(fake2.clone()).await;
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["presetId"], "feature");
    assert_eq!(
        fake2.presets.lock().unwrap()[0].id,
        "feature",
        "the resume launch uses the record's preset"
    );
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].session_id,
        lead
    );
}

/// (e) the nudge budget ending stalls with `NoProgress` while the tickets stay
/// `in_progress` (health, not blocked tickets).
#[tokio::test]
async fn stalled_is_health_not_blocked_ticket() {
    let f = Fixture::new();
    let a = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &a).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    d.tick().await.unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(status["leads"][0]["health"]["kind"], "no_progress");
    assert!(
        status["leads"][0]["health"]["nextProbeAt"].is_null(),
        "no_progress has no probe"
    );
    assert_eq!(status["needsYou"].as_array().unwrap().len(), 0);
    assert_eq!(f.task(&a).status, Status::InProgress);
    assert!(f.task(&a).run.is_some());
}

/// The one-time migration: claims become records, the continuations file is
/// folded in and removed, plumbing-blocked tickets move to BACKLOG with the
/// System note (owner override), and `projectLeads` keep feeding presets.
#[tokio::test]
async fn migration_from_continuations_and_plumbing_blocks() {
    let f = Fixture::new();
    let working = f.add("working", &[]);
    let blocked = f.add("blocked", &[]);
    let todo = f.add("todo ticket", &[]);
    // A lead claim (as the pre-OIN-80 dispatcher left it) + a continuation entry.
    unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &working, "legacy-lead-1")
        .unwrap();
    // A real preferences file always carries the built-in presets (Store::save
    // serializes the whole store); `feature` must resolve from it.
    let prefs_store = unipi_host::superagent_presets::Store {
        default_preset_id: "feature".into(),
        project_leads: std::iter::once((
            f.project.slug.clone(),
            unipi_host::superagent_presets::ProjectLead {
                model: Some("faux/migrated".into()),
                thinking: Some("low".into()),
            },
        ))
        .collect(),
        ..unipi_host::superagent_presets::Store::default()
    };
    let prefs = f.layout.home.join("superagent-preferences.json");
    std::fs::write(&prefs, serde_json::to_vec(&prefs_store).unwrap()).unwrap();
    std::fs::write(
        f.layout.home.join("dispatcher-continuations.json"),
        r#"{"legacy-lead-1":{"tickets":[],"sent":true,"total":1}}"#,
    )
    .unwrap();
    // A plumbing-blocked ticket (no run): the six-tickets shape of 2026-11-08.
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&blocked).unwrap();
    task.status = Status::Blocked;
    task.push_activity(
        chrono::Utc::now(),
        Actor::System,
        "blocked: Worker ended without a transition; move to todo to retry",
    );
    board.save(&task).unwrap();
    // A real question-blocked ticket stays blocked.
    let mut task = board.get(&todo).unwrap();
    task.status = Status::Blocked;
    task.push_activity(chrono::Utc::now(), Actor::System, "blocked: Which option?");
    board.save(&task).unwrap();

    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let leads = d.leads().lock().unwrap();
    assert_eq!(leads.version, 1);
    let record = &leads.leads[&f.project.slug];
    assert_eq!(record.session_id, "legacy-lead-1");
    assert_eq!(
        record.state,
        unipi_host::superagent_leads::LeadState::Working
    );
    assert_eq!(record.preset.id, "feature");
    assert_eq!(
        record.preset.model.as_deref(),
        Some("faux/migrated"),
        "projectLeads feed the migrated preset"
    );
    assert_eq!(record.continuation.total, 1);
    assert!(record.continuation.sent);
    drop(leads);
    assert!(
        !f.layout.home.join("dispatcher-continuations.json").exists(),
        "F13"
    );
    let blocked_task = f.task(&blocked);
    assert_eq!(
        blocked_task.status,
        Status::Backlog,
        "plumbing blocks go to backlog (owner override)"
    );
    assert!(blocked_task.run.is_none());
    assert!(blocked_task
        .activity
        .last()
        .unwrap()
        .text
        .contains("Blocked by host plumbing before OIN-80"));
    assert!(blocked_task
        .activity
        .last()
        .unwrap()
        .text
        .contains("Worker ended without a transition"));
    let todo_task = f.task(&todo);
    assert_eq!(
        todo_task.status,
        Status::Blocked,
        "a real question stays blocked"
    );
}

/// (Q11) archiving a lead's chat retires the lead and frees the slot.
#[tokio::test]
async fn archived_lead_chat_retires_and_frees_slot() {
    let f = Fixture::new();
    let a = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    let file = std::path::PathBuf::from(format!("/tmp/{lead}.jsonl"));
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .session_file = Some(file.clone());
    std::fs::write(&file, b"session").unwrap();
    d.sessions_archived(std::slice::from_ref(&lead)).await;
    std::fs::remove_file(&file).ok();
    assert!(
        d.leads().lock().unwrap().leads.is_empty(),
        "the record is gone"
    );
    assert_eq!(d.leads().lock().unwrap().retired.len(), 1);
    assert_eq!(d.leads().lock().unwrap().retired[0].cause, "archived");
    // The in_progress ticket went to backlog (Auto-start must not relaunch it).
    assert_eq!(f.task(&a).status, Status::Backlog);
    assert!(f.task(&a).run.is_none());
    // The next start mints a fresh id (the only path that may).
    let b = f.add("next", &[]);
    let next = d.start(&f.project.slug, &b).await.unwrap();
    assert_ne!(next, lead);
}

/// `superagent.lead_action` contract: the exact JSON the UI's `leadAction`
/// parses (`{ok:true}`) plus the error codes.
#[tokio::test]
async fn lead_action_contract() {
    let f = Fixture::new();
    let task = f.add("acted", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    let now = unipi_host::superagent_leads::now_ms();
    // wake on a working lead → E_LEAD_STATE.
    let err = unipi_host::superagent_leads::lead_action(&d, &lead, "wake", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_LEAD_STATE");
    assert!(err.message.contains("working"));
    // Unknown session → E_LEAD_UNKNOWN.
    let err = unipi_host::superagent_leads::lead_action(&d, "nope", "retry", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_LEAD_UNKNOWN");
    // switch_model without a model → E_SCHEMA; unknown model → E_UNKNOWN_MODEL.
    let err = unipi_host::superagent_leads::lead_action(&d, &lead, "switch_model", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_SCHEMA");
    let err = unipi_host::superagent_leads::lead_action(&d, &lead, "switch_model", Some("nope"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_SCHEMA");
    // A model outside the Fake's registry → E_UNKNOWN_MODEL (validation ran).
    let err =
        unipi_host::superagent_leads::lead_action(&d, &lead, "switch_model", Some("faux/absent"))
            .await
            .unwrap_err();
    assert_eq!(err.code, "E_UNKNOWN_MODEL");
    // Stall the lead, then retry works and replies {ok:true}.
    *fake.outcome.lock().unwrap() = Some("provider error: 429".into());
    d.tick_at(now).await.unwrap();
    let reply = unipi_host::superagent_leads::lead_action(&d, &lead, "retry", None)
        .await
        .unwrap();
    assert_eq!(reply, serde_json::json!({"ok": true}));
    assert!(fake
        .continues
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains("provider error"));
    assert_eq!(d.status().await.unwrap()["leads"][0]["state"], "working");
    // stop retires the lead.
    let reply = unipi_host::superagent_leads::lead_action(&d, &lead, "stop", None)
        .await
        .unwrap();
    assert_eq!(reply, serde_json::json!({"ok": true}));
    assert!(d.leads().lock().unwrap().leads.is_empty());
    assert_eq!(f.task(&task).status, Status::Backlog);
    // A stopped lead's action → E_LEAD_UNKNOWN (no record).
    let err = unipi_host::superagent_leads::lead_action(&d, &lead, "retry", None)
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_LEAD_UNKNOWN");
}

/// `lead:"new"` retires the current lead first; the only RPC that retires via start.
#[tokio::test]
async fn lead_new_starts_fresh_and_retires() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    let b = f.add("second", &[]);
    let next = d
        .start_tasks_with_lead(
            &f.project.slug,
            std::slice::from_ref(&b),
            None,
            unipi_host::superagent_presets::Overrides::default(),
            Some("new"),
        )
        .await
        .unwrap();
    assert_ne!(next, lead);
    let leads = d.leads().lock().unwrap();
    assert_eq!(leads.leads[&f.project.slug].session_id, next);
    assert_eq!(leads.retired.len(), 1, "lead:new retired the previous lead");
    assert_eq!(leads.retired[0].cause, "replaced");
    drop(leads);
    assert_eq!(
        f.task(&a).status,
        Status::Backlog,
        "the replaced lead's ticket went to backlog"
    );
}

/// R2: a wake racing a restart resume performs ONE launch, ONE continue.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wake_and_resume_race_one_launch() {
    let f = Fixture::new();
    let task = f.add("race resume", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.files.lock().unwrap() = [(lead.clone(), std::path::PathBuf::from("/tmp/race.jsonl"))]
        .into_iter()
        .collect();
    drop(d);
    let fake2 = Arc::new(Fake::default());
    let d = f.dispatcher(fake2.clone()).await;
    // After acquire's resume (one launch/continue already), a concurrent start
    // must not re-launch: the runtime is marked live by the resume.
    let (start, _) = tokio::join!(d.start(&f.project.slug, &task), d.tick());
    start.unwrap();
    assert_eq!(
        fake2.launches.load(Ordering::SeqCst),
        1,
        "resume + start = one launch"
    );
    assert_eq!(fake2.continues.lock().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// OIN-81: owner questions (B2)

fn lead_question(request_id: &str, text: &str) -> unipi_host::superagent_questions::LeadQuestion {
    unipi_host::superagent_questions::LeadQuestion {
        request_id: request_id.into(),
        texts: vec![text.into()],
        options: vec![vec!["postgres".into(), "sqlite".into()]],
        question_ids: vec!["q0".into()],
        allow_other: vec![true],
    }
}

/// A lead `ask_user` appears in superagent.status needsYou (kind question,
/// `in_progress`, requestId/options/askedAt/expiresAt) and the owner's answer
/// rides relay into the SAME lead session's turn; the ticket stays `in_progress`.
#[tokio::test]
async fn lead_question_in_needs_you_and_answer_reaches_same_turn() {
    let f = Fixture::new();
    let task = f.add("decide", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    // The lead's ask_user is open; the turn ended (waiting on the owner).
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-1", "T-1: postgres or sqlite?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    let status = d.status().await.unwrap();
    let needs = status["needsYou"].as_array().unwrap();
    assert_eq!(needs.len(), 1, "{status}");
    let row = &needs[0];
    assert_eq!(row["kind"], "question");
    assert_eq!(row["projectId"], f.project.slug);
    assert_eq!(row["taskId"], task);
    assert_eq!(row["status"], "in_progress");
    assert_eq!(row["question"], "T-1: postgres or sqlite?");
    assert_eq!(row["sessionId"], lead);
    assert_eq!(row["requestId"], "req-1");
    assert_eq!(row["options"], serde_json::json!([["postgres", "sqlite"]]));
    assert_eq!(row["askedAt"].as_i64(), Some(now));
    assert_eq!(row["expiresAt"].as_i64(), Some(now + 30 * 60_000));
    assert!(row.get("timeoutAt").is_none());
    // The ticket stays in_progress with its claim; the record is Waiting.
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(f.task(&task).run.as_ref().unwrap().session, lead);
    assert_eq!(status["leads"][0]["state"], "waiting");
    assert_eq!(status["leads"][0]["question"]["taskId"], task);
    // Nothing answered yet → a second relay (unrelated text) is a normal relay.
    // The owner answers THROUGH relay with the question's task id:
    let reply = d.relay(&f.project.slug, &task, "use sqlite").await.unwrap();
    assert_eq!(reply["delivered"], true);
    assert_eq!(reply["answered"], "question");
    assert_eq!(reply["sessionId"], lead);
    {
        let answers = fake.answers.lock().unwrap();
        assert_eq!(
            answers.len(),
            1,
            "the answer went into the SAME session's dialog"
        );
        assert_eq!(answers[0].0, "req-1");
        assert_eq!(answers[0].1, "use sqlite");
    }
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        1,
        "no new session for an answer"
    );
    // The question row is gone; the ticket never left in_progress; the answer
    // is recorded on the ticket.
    let status = d.status().await.unwrap();
    assert!(status["needsYou"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["taskId"] != task));
    assert_eq!(f.task(&task).status, Status::InProgress);
    let task = f.task(&task);
    assert!(
        task.activity
            .last()
            .unwrap()
            .text
            .contains("Owner answer: use sqlite"),
        "{:?}",
        task.activity.last().unwrap().text
    );
    assert_eq!(task.run.as_ref().unwrap().session, lead);
    // A second answer finds no question → E_ALREADY_ANSWERED.
    let err = d
        .relay(&f.project.slug, &task.id, "again")
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_ALREADY_ANSWERED");
}

/// The timeout converts the open question: ticket → blocked WITH the claim
/// kept, the dialog receives `TIMEOUT_TEXT`, the record's question clears.
#[tokio::test]
async fn question_timeout_blocks_ticket_and_keeps_claim() {
    let f = Fixture::new();
    let task = f.add("wait", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-1", "Which API key?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    assert_eq!(
        f.task(&task).status,
        Status::InProgress,
        "before the timeout the ticket stays in_progress"
    );
    // Exactly at the timeout the conversion happens.
    d.tick_at(now + 30 * 60_000).await.unwrap();
    let blocked = f.task(&task);
    assert_eq!(blocked.status, Status::Blocked);
    assert_eq!(
        blocked.run.as_ref().unwrap().session,
        lead,
        "the claim is KEPT"
    );
    assert!(
        blocked
            .activity
            .last()
            .unwrap()
            .text
            .starts_with("blocked: Which API key?"),
        "{:?}",
        blocked.activity.last().unwrap().text
    );
    {
        let answers = fake.answers.lock().unwrap();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].0, "req-1");
        assert_eq!(answers[0].1, "<skipped>", "dialog is resolved as skipped");
    }
    {
        let deliveries = fake.deliveries.lock().unwrap();
        let (steer_text, mode) = deliveries
            .iter()
            .find(|(t, _)| t.contains("30 min"))
            .expect("timeout guidance delivered as steer");
        assert_eq!(*mode, unipi_host::superagent::Delivery::Steer);
        assert!(
            steer_text.contains(&task),
            "the timeout text names the ticket"
        );
    }
    // The row is now the blocked question (same session id), still answerable.
    let status = d.status().await.unwrap();
    let needs = status["needsYou"].as_array().unwrap();
    assert_eq!(needs.len(), 1, "{status}");
    assert_eq!(needs[0]["kind"], "question");
    assert_eq!(needs[0]["status"], "blocked");
    assert_eq!(needs[0]["sessionId"], lead);
    assert_eq!(needs[0]["question"], "Which API key?");
}

/// A blocked-ticket answer is delivered to the same lead session (wake same
/// id, launches == 1 per wake), the ticket returns to `in_progress` under the
/// same claim.
#[tokio::test]
async fn answer_to_blocked_ticket_wakes_same_lead() {
    let f = Fixture::new();
    let task = f.add("blocked-question", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-1", "Which API key?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    // At the timeout the conversion happens (blocked, claim kept).
    d.tick_at(now + 30 * 60_000).await.unwrap();
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert_eq!(
        f.task(&task).run.as_ref().unwrap().session,
        lead,
        "claim kept"
    );
    // The lead's runtime is gone (warm unload / restart) while the question waits.
    fake.live.lock().unwrap().clear();
    let launches_before = fake.launches.load(Ordering::SeqCst);
    // The answer arrives through `superagent.answer` (same-session wake, branch 2).
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        d.answer_blocked_question(&lead, &task, "key is sk-123"),
    )
    .await
    .expect("answer does not hang")
    .unwrap();
    assert_eq!(reply["answered"], "blocked");
    assert_eq!(reply["sessionId"], lead, "the SAME session id");
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        launches_before + 1,
        "one wake launch"
    );
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(f.task(&task).run.as_ref().unwrap().session, lead);
    // The answer is a visible chat message in the SAME session (relay_delivery,
    // not the invisible host continue).
    let deliveries = fake.deliveries.lock().unwrap();
    let delivery = deliveries
        .iter()
        .find(|(text, _)| text.contains("Owner answer on"))
        .expect("the answer is delivered as a chat message");
    assert!(delivery.0.contains("sk-123"));
    assert!(
        delivery.0.contains("Which API key?"),
        "the delivery quotes the question: {}",
        delivery.0
    );
}

/// Plumbing texts never surface in needsYou (F7); real questions do.
#[tokio::test]
async fn plumbing_never_in_needs_you() {
    let f = Fixture::new();
    let task = f.add("parked", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    unipi_kanboard::dispatcher::block(
        &f.layout,
        f.project.clone(),
        &task,
        &lead,
        unipi_host::superagent::INTERRUPTED,
    )
    .unwrap();
    let status = d.status().await.unwrap();
    assert_eq!(status["needsYou"].as_array().unwrap().len(), 0, "{status}");
    // A REAL lead question (no plumbing prefix) still surfaces.
    let other = f.add("real-question", &[]);
    let lead2 = d
        .start_tasks(
            &f.project.slug,
            std::slice::from_ref(&other),
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    assert_eq!(lead2, lead);
    // Block it through the question path (approval_transition keeps the claim).
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &other,
        &lead,
        Some("Which storage engine?"),
    )
    .unwrap();
    let status = d.status().await.unwrap();
    let needs = status["needsYou"].as_array().unwrap();
    assert_eq!(needs.len(), 1, "{status}");
    assert_eq!(needs[0]["taskId"], other);
    assert_eq!(needs[0]["question"], "Which storage engine?");
    // Approval rows carry kind:"approval"; question rows kind:"question".
    assert_eq!(needs[0]["kind"], "question");
}

/// The lead's new question raises exactly ONE notification across its entire
/// lifetime (open -> repeated ticks -> timeout conversion).
#[tokio::test]
async fn question_notifications_fire_once() {
    let f = Fixture::new();
    let task = f.add("notify", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-1", "Which API key?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "one push when the question opens"
    );
    {
        let log = fake.notice_log.lock().unwrap();
        assert_eq!(log[0].0, lead);
        assert!(log[0].1.contains("Lead question"), "{}", log[0].1);
        assert_eq!(log[0].2, "Which API key?");
    }
    // Repeated ticks never re-notify the same question.
    for _ in 0..3 {
        d.tick_at(now + 1_000).await.unwrap();
    }
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "repeated ticks never re-notify"
    );
    assert_eq!(
        fake.answers.lock().unwrap().len(),
        0,
        "no timeout conversion before the deadline"
    );
    // Item 6: timeout conversion updates the card silently — still ONE push total.
    d.tick_at(now + 30 * 60_000).await.unwrap();
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "timeout must not emit a second push"
    );
}

/// The `superagent_status` tool and `superagent.status` wire contract: epoch-ms
/// numbers for `askedAt`, `expiresAt` and `notices[].at`, plus `superagent.answer`
/// resuming a blocked-claimed ticket even while the lead is still live.
#[tokio::test]
async fn superagent_status_payload_shape() {
    let f = Fixture::new();
    let task = f.add("status-tool", &[]);
    let other = f.add("other-work", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d
        .start_tasks(
            &f.project.slug,
            &[task.clone(), other.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-1", &format!("{task}: Which API key?")));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    // Wire status (`superagent.status`) emits epoch-ms numbers for question times and notice `at`.
    let wire = d.status().await.unwrap();
    let q_row = &wire["needsYou"][0];
    assert_eq!(q_row["kind"], "question");
    assert_eq!(q_row["status"], "in_progress");
    assert_eq!(q_row["askedAt"].as_i64(), Some(now));
    assert_eq!(q_row["expiresAt"].as_i64(), Some(now + 30 * 60_000));
    assert!(q_row.get("timeoutAt").is_none());
    assert_eq!(
        q_row["options"],
        serde_json::json!([["postgres", "sqlite"]])
    );
    let notices = wire["notices"].as_array().unwrap();
    assert_eq!(notices.len(), 1, "{wire}");
    assert_eq!(notices[0]["kind"], "board_created");
    assert!(
        notices[0]["at"].is_i64(),
        "notice `at` must be epoch-ms number: {}",
        notices[0]
    );

    let payload = unipi_host::superagent_questions::status_payload(&d, None)
        .await
        .unwrap();
    assert_eq!(payload["leads"][0]["sessionId"], lead);
    assert_eq!(payload["leads"][0]["state"], "waiting");
    assert_eq!(payload["leads"][0]["question"]["taskId"], task);
    assert_eq!(payload["needsYou"][0]["kind"], "question");
    assert_eq!(payload["review"].as_array().unwrap().len(), 0);
    let scoped = unipi_host::superagent_questions::status_payload(&d, Some(&f.project.slug))
        .await
        .unwrap();
    assert_eq!(scoped["leads"].as_array().unwrap().len(), 1);
    let empty = unipi_host::superagent_questions::status_payload(&d, Some("other"))
        .await
        .unwrap();
    assert_eq!(empty["leads"].as_array().unwrap().len(), 0);

    // Timeout converts `task` to blocked-with-claim; lead resumes streaming `other` (`live == true`).
    d.tick_at(now + 30 * 60_000).await.unwrap();
    *fake.outcome.lock().unwrap() = None;
    let reply = d
        .answer_blocked_question(&lead, &task, "use sk-live")
        .await
        .unwrap();
    assert_eq!(reply["answered"], "blocked");
    assert_eq!(reply["sessionId"], lead);
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert_eq!(f.task(&task).run.as_ref().unwrap().session, lead);
}

// ---------------------------------------------------------------------------
// OIN-80 QA audit (r-qa-b1-report) regression tests, findings 1-10

/// Finding 1: a corrupt `superagent-leads.json` is an ERROR, never a migration:
/// acquire fails with `E_SUPERAGENT_STORAGE`, the file is left untouched (no
/// claims-based reconstruction, no overwrite).
#[tokio::test]
async fn corrupt_lead_store_is_an_error_not_a_migration() {
    let f = Fixture::new();
    let a = f.add("held", &[]);
    // A pre-OIN-80 claim the destructive path would have turned into a record.
    unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &a, "legacy-lead").unwrap();
    let path = f.layout.home.join("superagent-leads.json");
    std::fs::write(&path, b"{ not json ]").unwrap();
    let fake = Arc::new(Fake::default());
    let Err(err) =
        Dispatcher::acquire(f.layout.clone(), f.home.path().join("config.json"), fake).await
    else {
        panic!("a corrupt store must fail acquire")
    };
    assert_eq!(err.code, "E_SUPERAGENT_STORAGE");
    assert!(
        err.message.contains("superagent-leads.json"),
        "{}",
        err.message
    );
    // The corrupt file was NOT overwritten by a migration.
    assert_eq!(std::fs::read(&path).unwrap(), b"{ not json ]");
    // The claim was not released either (no new chat may be minted from this).
    assert_eq!(f.task(&a).run.unwrap().session, "legacy-lead");
    // And a start during the outage cannot mint a new chat: the dispatcher
    // cannot even acquire, so there is no instance to start from.
    assert!(
        unipi_host::superagent::current().is_err() || {
            let d = unipi_host::superagent::current().unwrap();
            d.leads().lock().unwrap().leads.is_empty()
        }
    );
}

/// Finding 2: the plumbing migration revalidates under the board lock — a
/// ticket that gained a run (or a real question) between the scan and the
/// write keeps its state; a still-plumbing ticket moves.
#[tokio::test]
async fn migration_revalidates_tickets_under_board_lock() {
    let f = Fixture::new();
    let plumbing = f.add("plumbing", &[]);
    let raced = f.add("raced", &[]);
    // The raced ticket: a REAL claim kept on a blocked ticket (the
    // approval-transition shape) — the migration must leave it alone even
    // though its last block text is plumbing.
    unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &raced, "race-lead").unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &raced,
        "race-lead",
        Some("Worker ended without a transition"),
    )
    .unwrap();
    // The plain plumbing ticket (blocked, no run): moves to backlog.
    let board = Board::open(&f.layout, f.project.clone()).unwrap();
    let mut task = board.get(&plumbing).unwrap();
    task.status = Status::Blocked;
    task.push_activity(
        chrono::Utc::now(),
        Actor::System,
        "blocked: Worker ended without a transition",
    );
    board.save(&task).unwrap();

    let fake = Arc::new(Fake::default());
    let _d = f.dispatcher(fake.clone()).await; // acquire runs the migration
    let plumbing_task = f.task(&plumbing);
    assert_eq!(
        plumbing_task.status,
        Status::Backlog,
        "still-plumbing ticket moved"
    );
    assert!(plumbing_task.run.is_none());
    let raced_task = f.task(&raced);
    assert_eq!(
        raced_task.status,
        Status::Blocked,
        "a claimed ticket is never migrated"
    );
    assert_eq!(
        raced_task.run.unwrap().session,
        "race-lead",
        "the claim survives the migration"
    );
}

/// Finding 3: the REAL `HostWorkers` path — an empty-task launch (wake/probe)
/// must never index tasks[0]; it re-opens the existing session id, and a
/// zero-task launch with no session file refuses (no new chat).
#[tokio::test]
async fn host_workers_empty_task_launch_never_panics_or_mints() {
    use oino_core::{
        extensions::api::{ExtensionInstance, ExtensionMode},
        runtime::ToolSelection,
        services::{ResourceFlags, StaticLoader},
        testing::{FauxComponent, FauxProvider, FauxResponse},
    };
    use unipi_host::sessions::{HostHubOptions, SessionHub};
    use unipi_host::superagent::HostWorkers;
    let f = Fixture::new();
    let id = f.add("wake me", &[]);
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
            ..oino_core::services::ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(vec![
            Arc::new(FauxComponent(FauxProvider::new(vec![FauxResponse::Text(
                "ok".into(),
            )]))) as Arc<dyn ExtensionInstance>,
        ])),
        models: Some(models),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    let workers = Arc::new(HostWorkers::new(hub.clone(), f.layout.clone()));
    let mut preset = unipi_host::superagent_presets::Store::default()
        .resolve(
            Some("research"),
            unipi_host::superagent_presets::Overrides::default(),
        )
        .unwrap();
    preset.model = Some("faux/faux-1".into());
    let session_id = uuid::Uuid::now_v7().to_string();
    let task = unipi_kanboard::dispatcher::reserve(&f.layout, f.project.clone(), &id, &session_id)
        .unwrap();
    // A real launch with a task (creates the session file by reserved id).
    workers
        .launch_preset(&f.project, &task, &session_id, &preset)
        .await
        .unwrap();
    assert!(workers.live(&session_id).await);
    // Zero-task wake on a LIVE runtime: Ok, no panic, no second launch.
    workers
        .launch_lead(&f.project, &[], &session_id, &preset)
        .await
        .unwrap();
    // Stop the runtime, keep the session file: zero-task wake re-attaches.
    workers.stop(&session_id).await.unwrap();
    workers
        .launch_lead(&f.project, &[], &session_id, &preset)
        .await
        .unwrap();
    // Zero-task with NEITHER runtime nor session file: refused, not minted.
    let ghost = uuid::Uuid::now_v7().to_string();
    let err = workers
        .launch_lead(&f.project, &[], &ghost, &preset)
        .await
        .unwrap_err();
    assert!(err.contains("refusing to mint"), "{err}");
}

/// Finding 5: `archive_many` only retires leads of ids that were ACTUALLY
/// archived (`result.done`) — a rejected (running) lead keeps record + claims;
/// an id that matches no record is a no-op.
#[tokio::test]
async fn failed_batch_archive_keeps_the_leads_record() {
    let f = Fixture::new();
    let a = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    let file = std::path::PathBuf::from(format!("/tmp/{lead}.jsonl"));
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .session_file = Some(file.clone());
    std::fs::write(&file, b"session").unwrap();
    // The batch ARCHIVED a different chat and FAILED the lead's chat: only the
    // archived id reaches the retire hook (the hub passes result.done).
    d.sessions_archived(&["some-other-chat".to_owned()]).await;
    assert_eq!(
        d.leads().lock().unwrap().leads.len(),
        1,
        "a failed archive never retires the lead"
    );
    assert_eq!(f.task(&a).status, Status::InProgress, "the claim survives");
    // The archived id that matches a record still retires it (happy path).
    d.sessions_archived(std::slice::from_ref(&lead)).await;
    assert!(d.leads().lock().unwrap().leads.is_empty());
    assert_eq!(f.task(&a).status, Status::Backlog);
    std::fs::remove_file(&file).ok();
}

/// Finding 6: a storage failure during retirement propagates and the record
/// STAYS (no "stop succeeded with claims remaining"); the release write is
/// revalidated under the board lock (a raced claim is not clobbered).
#[tokio::test]
async fn retirement_with_failing_board_write_keeps_the_record() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let a = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    // Make every board write fail: the project's tasks dir loses write access
    // (write_atomic cannot create its temp file there).
    let tasks_dir = f.layout.tasks_dir(&f.project.slug);
    let readonly = std::fs::metadata(&tasks_dir).unwrap().permissions();
    let mut perms = readonly.clone();
    perms.set_mode(0o555);
    std::fs::set_permissions(&tasks_dir, perms).unwrap();
    let result = unipi_host::superagent_leads::retire_lead(&d, &f.project.slug, "stopped").await;
    let mut perms = readonly;
    perms.set_mode(0o755);
    std::fs::set_permissions(&tasks_dir, perms).unwrap();
    assert!(
        result.is_err(),
        "the release failure must surface (finding 6)"
    );
    // The record STAYED (not retired): no "stop succeeded with claims left".
    {
        let leads = d.leads().lock().unwrap();
        assert_eq!(
            leads.leads.len(),
            1,
            "the record survives the failed retirement"
        );
        assert_eq!(leads.retired.len(), 0);
    }
    // And the ticket kept its claim — the SAME session, never freed to a new chat.
    let task = f.task(&a);
    assert_eq!(task.status, Status::InProgress);
    assert_eq!(
        task.run.as_ref().map(|r| r.session.as_str()),
        Some(lead.as_str())
    );
    // A retry after the failure completes the retirement.
    let result = unipi_host::superagent_leads::retire_lead(&d, &f.project.slug, "stopped").await;
    assert!(result.is_ok());
    assert!(d.leads().lock().unwrap().leads.is_empty());
    assert_eq!(f.task(&a).status, Status::Backlog);
}

/// Finding 4 (integration): a realistic-epoch error text drives the probe —
/// "retry-after: 120" probes at now+120s, NOT at now+60s.
#[tokio::test]
async fn realistic_epoch_retry_after_schedules_the_probe() {
    let f = Fixture::new();
    let a = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &a).await.unwrap();
    *fake.outcome.lock().unwrap() =
        Some("provider error: 429 quota exhausted, retry-after: 120".into());
    let now = 1_794_391_219_566i64; // 2026-07-09T05:08:39Z
    d.tick_at(now).await.unwrap();
    // One minute later: no probe yet (the hint says 120 s).
    d.tick_at(now + 90_000).await.unwrap();
    assert_eq!(
        fake.continues.lock().unwrap().len(),
        0,
        "the 120 s hint defers the probe"
    );
    // At now+120 s (+ margin) the probe continues.
    d.tick_at(now + 121_000).await.unwrap();
    assert_eq!(
        fake.continues.lock().unwrap().len(),
        1,
        "probe at the hinted reset time"
    );
    assert!(fake.continues.lock().unwrap()[0].contains("provider error was"));
}

// ---------------------------------------------------------------------------
// OIN-83 lossless flow

/// The Superagent chat's tools stay cross-project-only, now including the
/// lead-model setter; the schema audit (fixtures) covers the shapes.
#[tokio::test]
async fn superagent_lead_model_tool_is_cross_project_and_validates() {
    use oino_core::extensions::api::ExtensionMode;
    use oino_core::runtime::ToolSelection;
    use oino_core::services::{ResourceFlags, StaticLoader};
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
            ..oino_core::services::ResourceFlags::default()
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
    let runtime = unipi_host::superagent_session::create(hub).await.unwrap();
    let session = runtime.session();
    let names = session
        .all_tools()
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>();
    assert!(
        names.iter().any(|n| n == "superagent_lead_model"),
        "{names:?}"
    );
    // The core allow-list admits it (dotted and underscored); memory_store too.
    for name in ["superagent_lead_model", "superagent.lead_model"] {
        let args =
            serde_json::from_value(serde_json::json!({"projectId":"test","model":"faux/example"}))
                .unwrap();
        assert!(
            session.check_permission(name, &args).await.is_ok(),
            "denied: {name}"
        );
    }
    let store = serde_json::json!({"title":"t","content":"c"});
    assert!(session
        .check_permission("memory_store", store.as_object().unwrap())
        .await
        .is_ok());
    let delete = serde_json::json!({"title":"t"});
    assert!(session
        .check_permission("memory_delete", delete.as_object().unwrap())
        .await
        .is_err());
    runtime.dispose().await;
}

/// The Superagent's `set_lead_model` RPC validates the model (`E_UNKNOWN_MODEL`),
/// switches the live lead through `lead_action`, and with remember saves the
/// project lead model so the next launch keeps it.
#[tokio::test]
async fn superagent_lead_model_switches_and_remembers() {
    let f = Fixture::new();
    let task = f.add("held", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    // Unknown model refuses with E_UNKNOWN_MODEL, nothing changes.
    let err = d
        .set_lead_model(&f.project.slug, "faux/missing", false)
        .await
        .unwrap_err();
    assert_eq!(err.code, "E_UNKNOWN_MODEL");
    assert!(fake.models.lock().unwrap().is_empty());
    assert!(f.task(&task).run.is_some(), "claim kept");
    // Valid model switches the live lead.
    let reply = d
        .set_lead_model(&f.project.slug, "faux/example", false)
        .await
        .unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(
        fake.models.lock().unwrap().last().map(String::as_str),
        Some("faux/example")
    );
    // remember: the project lead preset is updated, the next launch keeps it.
    let reply = d
        .set_lead_model(&f.project.slug, "faux/fallback", true)
        .await
        .unwrap();
    assert_eq!(reply["remember"], true);
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(f.layout.home.join("superagent-preferences.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved["projectLeads"][&f.project.slug]["model"],
        "faux/fallback"
    );
    // A fresh launch without overrides resolves through the project lead.
    let next = f.add("next", &[]);
    let id = d.start(&f.project.slug, &next).await.unwrap();
    assert_eq!(id, lead, "same lead session");
    let preset = d
        .leads()
        .lock()
        .unwrap()
        .for_project(&f.project.slug)
        .map(|r| r.preset.model.clone());
    assert_eq!(preset, Some(Some("faux/fallback".into())));
    drop(d);
}

/// The Superagent's `add` hands the new ticket to the project's lead in the
/// same step (result names the lead), stamps the Superagent origin, and a
/// parked `--status backlog` add does not dispatch.
#[tokio::test]
async fn superagent_add_hands_ticket_to_lead() {
    let f = Fixture::new();
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    // First add (no lead yet): the hand-over notice path (no lead to hand to).
    let mut common = Common::new(Actor::User, ChainGate::InReview);
    common.origin = Some(unipi_kanboard::model::Origin {
        kind: unipi_kanboard::model::OriginKind::Superagent,
        session: Some("chat-super-1".into()),
        message: Some("entry-9".into()),
    });
    let first = commands::add(
        &f.layout,
        f.project.clone(),
        &common,
        "From the chat",
        Some("the owner's exact words"),
        Some(Status::Backlog),
        Priority::Medium,
        &[],
        &[],
        &[],
        false,
    )
    .unwrap();
    let first_id = first["id"].as_str().unwrap().to_owned();
    // Start the lead on it (same-session hand-over in production goes through
    // the ChatTool; here we drive the dispatcher directly and assert the brief).
    // A live lead gets the brief as a queue delivery (a FRESH launch takes the
    // brief as its session prompt, which the Fake does not capture as text).
    let lead = d.start(&f.project.slug, &first_id).await.unwrap();
    let parked = commands::add(
        &f.layout,
        f.project.clone(),
        &common,
        "Parked",
        None,
        Some(Status::Backlog),
        Priority::Medium,
        &[],
        &[],
        &[],
        false,
    )
    .unwrap();
    let parked_id = parked["id"].as_str().unwrap().to_owned();
    let handed = d.start(&f.project.slug, &parked_id).await.unwrap();
    assert_eq!(handed, lead, "same lead chat");
    // The live-queue hand-over RELAYS the brief to the running lead (no second
    // launch): one launch total, the new ticket claimed by the SAME lead.
    assert_eq!(fake.batches.lock().unwrap().len(), 1, "no second launch");
    let text = fake
        .deliveries
        .lock()
        .unwrap()
        .last()
        .map(|(t, _)| t.clone())
        .unwrap_or_default();
    assert!(text.contains(&format!("## {parked_id} — Parked")), "{text}");
    assert!(
        text.contains("owner message entry-9 in Superagent chat chat-sup"),
        "{text}"
    );
    assert!(
        text.contains("read it word for word with owner_messages"),
        "{text}"
    );
    assert!(text.contains("Preset for this batch"), "{text}");
    // The origin survives the claim transition; the run names the lead session.
    let task = f.task(&parked_id);
    assert_eq!(
        task.origin.as_ref().map(|o| (o.kind, o.session.as_deref())),
        Some((
            unipi_kanboard::model::OriginKind::Superagent,
            Some("chat-super-1")
        ))
    );
    assert_eq!(
        task.run.as_ref().map(|r| r.session.clone()),
        Some(lead.clone()),
        "claimed by the same lead"
    );
    drop(d);
}

/// The lead's board tool is the host `WorkerBoard` WITH the lead role section
/// (the system prompt is built from the tool's guidelines at creation).
#[test]
fn lead_prompt_has_role_section() {
    let f = Fixture::new();
    let entry = unipi_host::superagent_session::worker_board(f.project.clone(), "lead-1".into());
    assert!(
        entry.prompt_snippet.is_some(),
        "the tool is listed in the prompt"
    );
    assert!(!entry.prompt_guidelines.is_empty(), "role section missing");
    let joined = entry.prompt_guidelines.join("\n");
    assert!(joined.contains("You are this project's lead"), "{joined}");
    assert!(joined.contains("owner_messages"), "{joined}");
    assert!(joined.contains("--attach"), "{joined}");
    assert!(joined.contains("backlog"), "{joined}");
    // No preset values in the frozen prefix (they travel in the brief).
    assert!(
        !joined.contains("maxParallel") && !joined.contains("swarm ×4"),
        "{joined}"
    );
    // The spec's test name is lead_prompt_has_role_section: the guidelines ARE
    // the prompt's role section (core renders tool_guidelines into the initial
    // system prompt via build_rules; the worker session declares this entry at
    // creation, so the section is in the frozen prefix and nowhere else).
}

/// `memory_store` passes the Superagent permission gate; `memory_delete` is
/// still refused (Q8). Driven through the REAL permission policy.
#[tokio::test]
async fn superagent_memory_store_allowed_delete_denied() {
    use oino_core::extensions::api::ExtensionMode;
    use oino_core::runtime::ToolSelection;
    use oino_core::services::{ResourceFlags, StaticLoader};
    use unipi_host::sessions::{HostHubOptions, SessionHub};
    let dir = tempfile::tempdir().unwrap();
    let memory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../extensions/dist/unipi.memory");
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
            ..oino_core::services::ResourceFlags::default()
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
    let runtime = unipi_host::superagent_session::create(hub).await.unwrap();
    let session = runtime.session();
    assert!(
        session
            .active_tool_names()
            .iter()
            .any(|n| n == "memory_store"),
        "memory_store declared"
    );
    // memory_delete may be DECLARED by the memory extension; the permission
    // gate asserted below is what refuses it.
    let store_args = serde_json::json!({"title":"oino_dispatch_prefs","content":"The owner wants opus for the oino project."});
    assert!(
        session
            .check_permission("memory_store", store_args.as_object().unwrap())
            .await
            .is_ok(),
        "memory_store must pass the gate"
    );
    let delete_args = serde_json::json!({"title":"oino_dispatch_prefs"});
    assert!(
        session
            .check_permission("memory_delete", delete_args.as_object().unwrap())
            .await
            .is_err(),
        "memory_delete must be refused"
    );
    runtime.dispose().await;
}

/// A lead-created follow-up ticket (kanboard add through the worker tool)
/// stamps the `lead:<session>` origin and lands in backlog (the role section's
/// follow-up rule), so the Superagent can schedule it.
#[tokio::test]
async fn lead_add_stamps_lead_origin() {
    let f = Fixture::new();
    // WorkerBoard's exec resolves the layout from the process env (no env
    // override in the request), so the temp board home must be exported.
    let previous_home = std::env::var_os("UNIPI_KANBOARD_HOME");
    std::env::set_var("UNIPI_KANBOARD_HOME", &f.layout.home);
    let layout = f.layout.clone();
    let project = f.project.clone();
    let session = "lead-session-42";
    let entry = unipi_host::superagent_session::worker_board(project.clone(), session.to_owned());
    let tool = entry.tool;
    let call: oino_ai::ToolCall = serde_json::from_value(serde_json::json!({
        "id":"call_add","name":"kanboard",
        "arguments":{"argv":["add","Follow-up fix","--body","found while working OIN-12"]}
    }))
    .unwrap();
    let invocation = oino_agent::ToolInvocation {
        tool_call_id: "call_add".into(),
        args: call.arguments.clone(),
        parent_tool_call_id: None,
    };
    let result = oino_agent::AgentTool::execute(
        tool.as_ref(),
        invocation,
        oino_agent::UpdateSink::discard(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    let text = match &result.content[0] {
        oino_ai::UserContent::Text(t) => &t.text,
        oino_ai::UserContent::Image(_) => panic!("text"),
    };
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    let id = value["id"].as_str().unwrap().to_owned();
    let task = f.task(&id);
    assert_eq!(
        task.status,
        Status::Backlog,
        "follow-up add lands in backlog"
    );
    assert_eq!(
        task.origin.as_ref().map(|o| (o.kind, o.session.as_deref())),
        Some((unipi_kanboard::model::OriginKind::Lead, Some(session)))
    );
    // The identity flags stay rejected.
    let bad: oino_ai::ToolCall = serde_json::from_value(serde_json::json!({
        "id":"call_bad","name":"kanboard",
        "arguments":{"argv":["add","Hijack","--origin","chat:evil"]}
    }))
    .unwrap();
    let invocation = oino_agent::ToolInvocation {
        tool_call_id: "call_bad".into(),
        args: bad.arguments.clone(),
        parent_tool_call_id: None,
    };
    assert!(oino_agent::AgentTool::execute(
        tool.as_ref(),
        invocation,
        oino_agent::UpdateSink::discard(),
        tokio_util::sync::CancellationToken::new()
    )
    .await
    .is_err());
    drop(layout);
    drop(project);
    match previous_home {
        Some(value) => std::env::set_var("UNIPI_KANBOARD_HOME", value),
        None => std::env::remove_var("UNIPI_KANBOARD_HOME"),
    }
}

/// `owner_messages` pages the owner's verbatim messages of a chat that sent
/// work to the lead's project, excluding host-authored entries (the unit-level
/// behavior lives in `superagent_owner_messages`; this drives the real tool).
#[tokio::test]
async fn owner_messages_tool_pages_verbatim_from_real_session_file() {
    let f = Fixture::new();
    let sessions_root = f.home.path().join("sessions");
    let chat_dir = sessions_root.join("--home-oi-.oino-superagent");
    std::fs::create_dir_all(&chat_dir).unwrap();
    let entry = |id: &str, text: &str, author: Option<&str>| {
        let message = oino_ai::UserMessage {
            author: author.map(str::to_owned),
            content: oino_ai::UserContentField::Text(text.to_owned()),
            timestamp: 5_000,
        };
        oino_session::SessionEntry {
            id: oino_session::entry::EntryId::new(id),
            parent_id: None,
            timestamp: oino_session::entry::IsoTimestamp::from_ms(1_000),
            compaction_summary_id: None,
            kind: oino_session::entry::EntryKind::Message {
                message: oino_ai::AgentMessage::User(message),
            },
        }
    };
    let mut text = format!("{{\"type\":\"session\",\"version\":3,\"id\":\"chat-real-01\",\"timestamp\":\"2026-11-11T08:00:00.000Z\",\"cwd\":\"{}\"}}\n", f.home.path().display());
    for e in [
        entry(
            "e1",
            "please fix the login timeout, it repros every night",
            None,
        ),
        entry("e2", "relayed to the lead", Some("superagent")),
        entry(
            "e3",
            "also the dashboard chart is blank after midnight",
            None,
        ),
    ] {
        text.push_str(&serde_json::to_string(&e).unwrap());
        text.push('\n');
    }
    std::fs::write(
        chat_dir.join("20261111T080000-000Z_chat-real-01.jsonl"),
        text,
    )
    .unwrap();
    let mut common = Common::new(Actor::User, ChainGate::InReview);
    common.origin = Some(unipi_kanboard::model::Origin {
        kind: unipi_kanboard::model::OriginKind::Superagent,
        session: Some("chat-real-01".into()),
        message: Some("e1".into()),
    });
    let id = commands::add(
        &f.layout,
        f.project.clone(),
        &common,
        "From the chat",
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
    let d = f.dispatcher(fake).await;
    let lead = d.start(&f.project.slug, &id).await.unwrap();
    drop(d);
    // The tool call as the lead would make it (default chatId = newest origin).
    let sessions_root = sessions_root.clone();
    let project = f.project.clone();
    let payload = unipi_host::superagent_owner_messages::page(
        &f.layout,
        &project,
        None,
        None,
        None,
        20,
        &sessions_root,
    )
    .await
    .unwrap();
    assert_eq!(payload["chatId"], "chat-real-01");
    assert_eq!(payload["total"], 2);
    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(
        messages[0]["text"],
        "also the dashboard chart is blank after midnight"
    );
    assert_eq!(
        messages[1]["text"],
        "please fix the login timeout, it repros every night"
    );
    assert!(
        messages.iter().all(|m| m["text"] != "relayed to the lead"),
        "host-authored excluded"
    );
    let _ = lead;
}

// ---------------------------------------------------------------------------
// r-qa-leads-2-report regression tests (items 1-8, 11, 13a)

/// Item 1 (P0): a LEAD-STORE save failure during retirement leaves the
/// in-memory record intact so a subsequent start reuses the same chat and
/// never mints a new session.
#[tokio::test]
async fn retirement_with_failing_lead_store_save_keeps_in_memory_record_and_reuses_chat() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let b = f.add("second", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();

    // Inject a LEAD-STORE save failure (board writes still succeed!): replace
    // `superagent-leads.json` with a non-empty directory so atomic rename onto
    // `leads_path` fails deterministically with EISDIR.
    let leads_path = f.layout.home.join("superagent-leads.json");
    let saved_bytes = std::fs::read(&leads_path).unwrap();
    std::fs::remove_file(&leads_path).unwrap();
    std::fs::create_dir_all(leads_path.join("blocker")).unwrap();

    let err = unipi_host::superagent_leads::retire_lead(&d, &f.project.slug, "stopped")
        .await
        .expect_err("lead-store save failure must propagate");
    assert_eq!(err.code, "E_SUPERAGENT_STORAGE");

    // In-memory record and lead_sessions registration remain intact!
    {
        let store = d.leads().lock().unwrap();
        assert_eq!(
            store
                .leads
                .get(&f.project.slug)
                .map(|r| r.session_id.as_str()),
            Some(lead.as_str()),
            "in-memory LeadRecord must survive a failed store save"
        );
        assert!(store.retired.is_empty());
    }
    assert!(d.is_lead_session(&lead));

    // Restore `superagent-leads.json` and start `b`: reuses `lead`, never mints a new chat.
    std::fs::remove_dir_all(&leads_path).unwrap();
    std::fs::write(&leads_path, &saved_bytes).unwrap();
    let started = d.start(&f.project.slug, &b).await.unwrap();
    assert_eq!(
        started, lead,
        "subsequent start must reuse the surviving lead chat"
    );
}

/// Item 2 (P1): blocked-answer delivery wakes/delivers FIRST; delivery failure
/// leaves the ticket `blocked` and retryable (`E_ALREADY_ANSWERED` only once
/// no longer blocked), and a post-delivery board failure surfaces a clear error
/// without losing the already-delivered prompt.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn blocked_answer_delivers_first_and_handles_delivery_and_board_failures() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let task = f.add("blocked-q", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task,
        &lead,
        Some("Use Redis or Valkey?"),
    )
    .unwrap();
    fake.live.lock().unwrap().clear();

    // 1. Wake/launch failure: ticket stays `blocked`, no `Owner answer:` written.
    fake.fail_launch.store(true, Ordering::SeqCst);
    let err = d
        .answer_blocked_question(&lead, &task, "Valkey")
        .await
        .expect_err("wake failure must return error");
    assert_eq!(err.code, "E_WORKER_LAUNCH");
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert!(f
        .task(&task)
        .activity
        .iter()
        .all(|a| !a.text.starts_with("Owner answer:")));

    // 2. Relay delivery failure: ticket STILL stays `blocked` and retryable.
    fake.fail_launch.store(false, Ordering::SeqCst);
    fake.fail_relay.store(true, Ordering::SeqCst);
    let err = d
        .answer_blocked_question(&lead, &task, "Valkey")
        .await
        .expect_err("relay failure must return error");
    assert_eq!(err.code, "E_RELAY_DELIVERY");
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert!(f
        .task(&task)
        .activity
        .iter()
        .all(|a| !a.text.starts_with("Owner answer:")));

    // 3. Retry after transient delivery failure succeeds and moves ticket `blocked -> in_progress`.
    fake.fail_relay.store(false, Ordering::SeqCst);
    let ok = d
        .answer_blocked_question(&lead, &task, "Valkey")
        .await
        .unwrap();
    assert_eq!(ok["delivered"], true);
    assert_eq!(ok["sessionId"], lead);
    assert_eq!(f.task(&task).status, Status::InProgress);
    assert!(f
        .task(&task)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: Valkey"));

    // Only NOW that the ticket is no longer `blocked` does a duplicate return `E_ALREADY_ANSWERED`.
    let dup = d
        .answer_blocked_question(&lead, &task, "Valkey again")
        .await
        .expect_err("non-blocked ticket returns E_ALREADY_ANSWERED");
    assert_eq!(dup.code, "E_ALREADY_ANSWERED");

    // 4. Post-delivery board-move failure: answer is delivered to the lead, and a
    // clear board error is returned without losing the prompt.
    let task2 = f.add("blocked-board-fail", &[]);
    d.start(&f.project.slug, &task2).await.unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task2,
        &lead,
        Some("Region?"),
    )
    .unwrap();
    let tasks_dir = f.layout.tasks_dir(&f.project.slug);
    let orig = std::fs::metadata(&tasks_dir).unwrap().permissions();
    let mut ro = orig.clone();
    ro.set_mode(0o555);
    std::fs::set_permissions(&tasks_dir, ro).unwrap();
    let board_err = d
        .answer_blocked_question(&lead, &task2, "eu-central-1")
        .await
        .expect_err("board write failure after delivery must surface");
    std::fs::set_permissions(&tasks_dir, orig).unwrap();
    assert_eq!(board_err.code, "E_BOARD_RESUME_FAILED");
    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(text, _)| text.contains("eu-central-1"))
            .count(),
        1,
        "the answer is delivered once before the board write failed"
    );
    assert!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .pending_answers
            .first()
            .is_some_and(|pa| pa.delivered && !pa.board_done),
        "outbox records delivered=true, board_done=false"
    );

    // 5. N1: retrying the same blocked answer skips delivery (delivered == true)
    // and finishes only the board move; total deliveries stay at EXACTLY ONE.
    let retry_ok = d
        .answer_blocked_question(&lead, &task2, "eu-central-1")
        .await
        .expect("retry must finish board move without re-delivering");
    assert_eq!(retry_ok["delivered"], true);
    assert_eq!(retry_ok["sessionId"], lead);
    assert_eq!(f.task(&task2).status, Status::InProgress);
    assert!(f
        .task(&task2)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: eu-central-1"));
    assert!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .pending_answers
            .is_empty(),
        "outbox clears once board_done succeeds"
    );
    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(text, _)| text.contains("eu-central-1"))
            .count(),
        1,
        "retry after post-delivery board failure must NOT deliver the answer twice"
    );
}

/// Item 3 (P1): durable timeout-in-progress marker retries a failed dialog
/// resolution on the next tick without losing attribution or re-blocking.
#[tokio::test]
async fn question_timeout_acknowledgment_retries_failed_dialog_resolution_idempotently() {
    let f = Fixture::new();
    let task = f.add("retry-ack", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-ack", "Which region?"));

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1);

    // At timeout: board transition succeeds, but `workers.answer` fails once.
    fake.fail_answer_once.store(true, Ordering::SeqCst);
    d.tick_at(now + 30 * 60_000).await.unwrap();
    assert_eq!(
        f.task(&task).status,
        Status::Blocked,
        "board parked on first timeout tick"
    );
    assert_eq!(f.task(&task).run.as_ref().unwrap().session, lead);
    assert_eq!(
        fake.dialog.lock().unwrap().len(),
        1,
        "dialog still open after failed answer"
    );
    {
        let store = d.leads().lock().unwrap();
        let pq = store.leads[&f.project.slug]
            .question
            .as_ref()
            .expect("durable PendingQuestion kept until dialog resolution succeeds");
        assert!(pq.timing_out);
        assert!(pq.board_blocked);
        assert!(!pq.dialog_resolved);
        assert_eq!(pq.first_seen_at, now);
    }

    // Next tick completes dialog resolution + steer delivery idempotently even
    // though `task` is now `Blocked` rather than `InProgress`.
    let blocked_entries = f
        .task(&task)
        .activity
        .iter()
        .filter(|a| a.text.starts_with("blocked:"))
        .count();
    d.tick_at(now + 30 * 60_000 + 60_000).await.unwrap();
    assert!(
        fake.dialog.lock().unwrap().is_empty(),
        "dialog resolved on retry tick"
    );
    assert!(d.leads().lock().unwrap().leads[&f.project.slug]
        .question
        .is_none());
    assert_eq!(
        f.task(&task)
            .activity
            .iter()
            .filter(|a| a.text.starts_with("blocked:"))
            .count(),
        blocked_entries,
        "board transition was not duplicated"
    );
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "still exactly one push"
    );
}

/// Item 4 (P1): end-to-end answer encoding for (a) option match, (b) `other:false`
/// non-match, (c) multi-question, and (d) timeout on `other:false`.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn question_answer_encoding_covers_option_match_restricted_multi_and_timeout() {
    let f = Fixture::new();
    let task = f.add("encode", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    let now = unipi_host::superagent_leads::now_ms();

    // (a) 1 question (`other: false`) + exact option match -> `selected: ["postgres"]`, no steer.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-a".into(),
            texts: vec!["Database?".into()],
            options: vec![vec!["postgres".into(), "sqlite".into()]],
            question_ids: vec!["q0".into()],
            allow_other: vec![false],
        });
    d.tick_at(now).await.unwrap();
    let deliveries_before = fake.deliveries.lock().unwrap().len();
    d.relay(&f.project.slug, &task, "postgres").await.unwrap();
    assert_eq!(
        fake.raw_answers.lock().unwrap().last().unwrap(),
        &(
            "req-a".to_owned(),
            oino_core::ui::UiAnswer::Value(
                serde_json::json!({"answers": [{"id": "q0", "selected": ["postgres"]}]})
            )
        )
    );
    assert_eq!(
        fake.deliveries.lock().unwrap().len(),
        deliveries_before,
        "no steer needed"
    );

    // (b) 1 question (`other: false`) + non-matching text -> skipped + verbatim steer.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-b".into(),
            texts: vec!["Database?".into()],
            options: vec![vec!["postgres".into(), "sqlite".into()]],
            question_ids: vec!["q0".into()],
            allow_other: vec![false],
        });
    d.tick_at(now + 1_000).await.unwrap();
    d.relay(&f.project.slug, &task, "cockroachdb with TLS")
        .await
        .unwrap();
    assert_eq!(
        fake.raw_answers.lock().unwrap().last().unwrap(),
        &(
            "req-b".to_owned(),
            oino_core::ui::UiAnswer::Value(serde_json::json!({"answers": []}))
        )
    );
    assert_eq!(
        fake.deliveries.lock().unwrap().last().unwrap(),
        &(
            "Owner answered your question (\"Database?\"): cockroachdb with TLS".to_owned(),
            unipi_host::superagent::Delivery::Steer,
        )
    );

    // (c) Multi-question dialog -> skipped + verbatim steer.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-c".into(),
            texts: vec!["Database?".into(), "Backup cadence?".into()],
            options: vec![
                vec!["postgres".into()],
                vec!["hourly".into(), "daily".into()],
            ],
            question_ids: vec!["q0".into(), "q1".into()],
            allow_other: vec![true, true],
        });
    d.tick_at(now + 2_000).await.unwrap();
    d.relay(&f.project.slug, &task, "postgres, hourly")
        .await
        .unwrap();
    assert_eq!(
        fake.raw_answers.lock().unwrap().last().unwrap(),
        &(
            "req-c".to_owned(),
            oino_core::ui::UiAnswer::Value(serde_json::json!({"answers": []}))
        )
    );
    assert_eq!(
        fake.deliveries.lock().unwrap().last().unwrap(),
        &(
            "Owner answered your question (\"1) Database? 2) Backup cadence?\"): postgres, hourly"
                .to_owned(),
            unipi_host::superagent::Delivery::Steer,
        )
    );

    // (d) Timeout on `other: false` question -> skipped + timeout steer.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-d".into(),
            texts: vec!["Strict choice?".into()],
            options: vec![vec!["yes".into(), "no".into()]],
            question_ids: vec!["q0".into()],
            allow_other: vec![false],
        });
    d.tick_at(now + 3_000).await.unwrap();
    d.tick_at(now + 3_000 + 30 * 60_000).await.unwrap();
    assert_eq!(
        fake.raw_answers.lock().unwrap().last().unwrap(),
        &(
            "req-d".to_owned(),
            oino_core::ui::UiAnswer::Value(serde_json::json!({"answers": []}))
        )
    );
    let (steer_msg, mode) = fake.deliveries.lock().unwrap().last().cloned().unwrap();
    assert_eq!(mode, unipi_host::superagent::Delivery::Steer);
    assert!(
        steer_msg.contains("30 min") && steer_msg.contains(&task),
        "{steer_msg}"
    );
}

/// Item 5 (P1): stale answer carrying an old session id or old request id is
/// rejected under the transaction lock without touching a replacement lead.
#[tokio::test]
async fn stale_answer_rejected_under_lock_without_touching_replacement_lead() {
    let f = Fixture::new();
    let task = f.add("race-stale", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead_1 = d.start(&f.project.slug, &task).await.unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task,
        &lead_1,
        Some("Old lead question?"),
    )
    .unwrap();

    // Stop `lead_1` and start replacement `lead_2` (`lead: "new"`), which claims `task`
    // and asks its OWN question on `task`.
    d.stop(&f.project.slug, &task).await.unwrap();
    let lead_2 = d
        .start_tasks_with_lead(
            &f.project.slug,
            std::slice::from_ref(&task),
            None,
            unipi_host::superagent_presets::Overrides::default(),
            Some("new"),
        )
        .await
        .unwrap();
    assert_ne!(lead_1, lead_2);
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-new", "Replacement lead question?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    let deliveries_before = fake.deliveries.lock().unwrap().len();

    // Stale `superagent.answer` aimed at `lead_1` is rejected with `E_LEAD_UNKNOWN`
    // and never touches `lead_2` or its open dialog.
    let err = d
        .answer_blocked_question(&lead_1, &task, "stale answer for lead 1")
        .await
        .expect_err("stale session id must be rejected");
    assert_eq!(err.code, "E_LEAD_UNKNOWN");
    assert_eq!(fake.answers.lock().unwrap().len(), 0);
    assert_eq!(fake.deliveries.lock().unwrap().len(), deliveries_before);

    // Stale open-question `relay_to_lead_checked` with `lead_1` -> `E_LEAD_UNKNOWN`.
    let err = d
        .relay_to_lead_checked(
            &f.project.slug,
            "stale open answer",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead_1),
            Some("req-old"),
        )
        .await
        .expect_err("stale relay session must be rejected");
    assert_eq!(err.code, "E_LEAD_UNKNOWN");

    // Stale `request_id` on `lead_2` -> `E_ALREADY_ANSWERED`, leaving `req-new` untouched.
    let err = d
        .relay_to_lead_checked(
            &f.project.slug,
            "stale request id answer",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead_2),
            Some("req-old"),
        )
        .await
        .expect_err("stale request id must be rejected");
    assert_eq!(err.code, "E_ALREADY_ANSWERED");
    assert_eq!(fake.dialog.lock().unwrap().len(), 1, "req-new remains open");
}

/// Item 7 (P1): `stop -> reuse -> launch` registers the reused session id in
/// `lead_sessions` BEFORE the runtime launches so turn-end `Settled` pushes stay suppressed.
#[tokio::test]
async fn reused_lead_registers_before_launch_and_keeps_turn_end_suppression() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let b = f.add("second", &[]);
    let session_file = f.home.path().join("reused-lead.jsonl");
    std::fs::write(&session_file, b"{}\n").unwrap();

    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let _ = fake.dispatcher_weak.set(Arc::downgrade(&d));

    let lead = d.start(&f.project.slug, &a).await.unwrap();
    *fake.files.lock().unwrap() = [(lead.clone(), session_file.clone())].into_iter().collect();
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .session_file = Some(session_file);
    assert!(d.is_lead_session(&lead));

    // Stop retires `lead` and removes it from `lead_sessions`.
    d.stop(&f.project.slug, &a).await.unwrap();
    assert!(!d.is_lead_session(&lead));

    // Reuse (`lead: "reuse"`) reopens the SAME session id and registers it
    // BEFORE `launch_lead` executes.
    fake.registered_at_launch.lock().unwrap().clear();
    let reused = d
        .start_tasks_with_lead(
            &f.project.slug,
            std::slice::from_ref(&b),
            None,
            unipi_host::superagent_presets::Overrides::default(),
            Some("reuse"),
        )
        .await
        .unwrap();
    assert_eq!(reused, lead);
    assert!(
        d.is_lead_session(&lead),
        "reused lead must suppress turn-end push"
    );
    assert_eq!(
        fake.registered_at_launch.lock().unwrap().as_slice(),
        &[(lead, true)],
        "session id must be in lead_sessions BEFORE runtime runs"
    );
}

/// Item 8 (P1): initial launch failure routes through deduplicated stall
/// notification, sets `notified = true` only after the notice was sent, and a
/// subsequent failed probe does NOT send a second notice.
#[tokio::test]
async fn initial_launch_failure_sends_one_stall_notice_and_failed_probe_does_not_renotify() {
    let f = Fixture::new();
    let a = f.add("launch-fail", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;

    // If notice dispatch itself fails (`false`), `notified` stays `false`.
    fake.fail_launch.store(true, Ordering::SeqCst);
    fake.fail_notice.store(true, Ordering::SeqCst);
    let err = d.start(&f.project.slug, &a).await.unwrap_err();
    assert_eq!(err.code, "E_WORKER_LAUNCH");
    assert_eq!(fake.notices.load(Ordering::SeqCst), 0);
    assert!(
        !d.leads().lock().unwrap().leads[&f.project.slug]
            .health
            .as_ref()
            .unwrap()
            .notified,
        "notified must remain false when no notice was sent"
    );

    // Reopen attempt with working notice dispatch sends ONE notice and sets `notified = true`.
    fake.fail_notice.store(false, Ordering::SeqCst);
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now + unipi_host::superagent_leads::STALL_REPROBE_MS + 1_000)
        .await
        .unwrap();
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "one stall notice sent"
    );
    assert!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .health
            .as_ref()
            .unwrap()
            .notified
    );

    // Another failed probe afterwards increments attempts without sending a second notice!
    d.tick_at(now + 2 * unipi_host::superagent_leads::STALL_REPROBE_MS + 2_000)
        .await
        .unwrap();
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "failed probe must not send a second notice"
    );
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .health
            .as_ref()
            .unwrap()
            .attempts,
        3
    );

    // Also verify fresh initial launch failure directly on a second project:
    // 1 notice on initial failure, 0 extra on failed probe.
    let f2 = Fixture::new();
    let b = f2.add("fresh-launch-fail", &[]);
    let fake2 = Arc::new(Fake::default());
    fake2.fail_launch.store(true, Ordering::SeqCst);
    let d2 = f2.dispatcher(fake2.clone()).await;
    let err2 = d2.start(&f2.project.slug, &b).await.unwrap_err();
    assert_eq!(err2.code, "E_WORKER_LAUNCH");
    assert_eq!(
        fake2.notices.load(Ordering::SeqCst),
        1,
        "initial launch failure sends 1 notice"
    );
    let probe_at = d2.leads().lock().unwrap().leads[&f2.project.slug]
        .health
        .as_ref()
        .unwrap()
        .next_probe_at
        .unwrap();
    d2.tick_at(probe_at + 1_000).await.unwrap();
    assert_eq!(
        fake2.notices.load(Ordering::SeqCst),
        1,
        "failed probe sends no second notice"
    );
}

/// Item 11 (P2): when automatic fallback `set_model` fails at runtime, do not
/// commit `on_fallback: true` or send continuation; stall the lead with a
/// reason naming the model error and never create a new chat.
#[tokio::test]
async fn fallback_model_switch_failure_stalls_without_committing_on_fallback_or_continuing() {
    let f = Fixture::new();
    let task = f.add("fallback-fail", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    d.configure(Config {
        enabled: false,
        lead: unipi_host::superagent_leads::LeadSettings {
            retries: 3,
            fallback_model: Some("faux/fallback".into()),
            ..unipi_host::superagent_leads::LeadSettings::default()
        },
        ..Config::default()
    })
    .await
    .unwrap();

    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.fail_set_model.lock().unwrap() = Some("model credentials revoked".into());
    *fake.outcome.lock().unwrap() = Some("provider error: 429 quota exhausted".into());

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    let store = d.leads().lock().unwrap().leads[&f.project.slug].clone();
    assert_eq!(store.session_id, lead, "same chat kept");
    assert!(
        !store.on_fallback,
        "on_fallback must not commit when set_model failed"
    );
    assert_eq!(
        store.state,
        unipi_host::superagent_leads::LeadState::Stalled
    );
    let err_text = &store.health.as_ref().unwrap().error;
    assert!(
        err_text.contains("faux/fallback") && err_text.contains("model credentials revoked"),
        "health error must name the model switch failure: {err_text}"
    );
    assert!(
        fake.continues.lock().unwrap().is_empty(),
        "continuation must not be sent"
    );
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1);

    // Persisted JSON also keeps `onFallback: false`.
    let persisted = unipi_host::superagent_leads::LeadStore::load_or_migrate(&f.layout).unwrap();
    assert!(!persisted.leads[&f.project.slug].on_fallback);
}

/// Item 13a: a missing session file without positive archive evidence keeps
/// the lead record and stalls it with `'chat file missing'`; once archive
/// evidence appears in `<agent_dir>/trash`, the lead retires with `'archived'`.
#[tokio::test]
async fn missing_session_file_stalls_until_positive_archive_evidence() {
    let f = Fixture::new();
    let task = f.add("missing-file", &[]);
    let agent_dir = f.home.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let fake = Arc::new(Fake {
        agent_dir: Some(agent_dir.clone()),
        ..Fake::default()
    });
    let session_file = f.home.path().join("sessions/lead-chat.jsonl");
    std::fs::create_dir_all(session_file.parent().unwrap()).unwrap();
    std::fs::write(&session_file, b"{}\n").unwrap();

    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .session_file = Some(session_file.clone());

    // Delete session file WITHOUT putting it in trash (not archived!).
    std::fs::remove_file(&session_file).unwrap();
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    {
        let store = d.leads().lock().unwrap();
        let rec = store
            .leads
            .get(&f.project.slug)
            .expect("missing file without archive evidence must NOT retire the lead");
        assert_eq!(rec.session_id, lead);
        assert_eq!(rec.state, unipi_host::superagent_leads::LeadState::Stalled);
        assert_eq!(rec.health.as_ref().unwrap().error, "chat file missing");
        assert!(store.retired.is_empty());
    }
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1);

    // Status surfaces the stalled lead health card (`chat file missing`), and
    // repeated ticks neither retire nor re-notify.
    let status = d.status().await.unwrap();
    assert_eq!(status["leads"][0]["state"], "stalled");
    assert_eq!(status["leads"][0]["health"]["error"], "chat file missing");
    d.tick_at(now + 5_000).await.unwrap();
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1);

    // Same basename in `<agent_dir>/trash/` with a DIFFERENT session header id
    // must NOT retire the lead (N4).
    let trash = agent_dir.join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(
        trash.join("lead-chat.jsonl"),
        "{\"type\":\"session\",\"version\":3,\"id\":\"impostor-id\",\"timestamp\":\"2026-11-11T00:00:00.000Z\",\"cwd\":\"/tmp\"}\n",
    )
    .unwrap();
    d.tick_at(now + 8_000).await.unwrap();
    assert!(
        d.leads()
            .lock()
            .unwrap()
            .leads
            .contains_key(&f.project.slug),
        "basename-only match with mismatched header id must not retire the lead"
    );

    // Exact header id match in `<agent_dir>/trash/` retires the lead cleanly.
    std::fs::write(
        trash.join("lead-chat.jsonl"),
        format!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"{lead}\",\"timestamp\":\"2026-11-11T00:00:00.000Z\",\"cwd\":\"/tmp\"}}\n"
        ),
    )
    .unwrap();
    d.tick_at(now + 10_000).await.unwrap();
    let store = d.leads().lock().unwrap();
    assert!(store.leads.is_empty());
    assert_eq!(store.retired.len(), 1);
    assert_eq!(store.retired[0].cause, "archived");
}

// ---------------------------------------------------------------------------
// r-qa-leads-3-report regression tests (N2, N3, N5, D1, D2)

/// N2 (P1): restricted/multi-question answer stages verbatim text before closing
/// the dialog; if steer fails after dialog closure, both `lead_tick` and owner
/// retry resume from the missing delivery step without re-answering the closed
/// dialog or returning `E_ALREADY_ANSWERED`, delivering verbatim text once.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn restricted_and_multi_question_steer_failure_recovers_via_outbox_without_double_delivery() {
    let f = Fixture::new();
    let task = f.add("outbox-steer", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    let now = unipi_host::superagent_leads::now_ms();

    // Case A: restricted single-question -> dialog closes, steer fails -> next tick delivers once.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-n2-tick".into(),
            texts: vec!["Database?".into()],
            options: vec![vec!["postgres".into(), "sqlite".into()]],
            question_ids: vec!["q0".into()],
            allow_other: vec![false],
        });
    d.tick_at(now).await.unwrap();

    fake.fail_relay.store(true, Ordering::SeqCst);
    let err = d
        .relay_to_lead_checked(
            &f.project.slug,
            "use cockroachdb cluster",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead),
            Some("req-n2-tick"),
        )
        .await
        .expect_err("steer failure after dialog closure must surface E_RELAY_DELIVERY");
    assert_eq!(err.code, "E_RELAY_DELIVERY");
    assert!(fake.dialog.lock().unwrap().is_empty(), "dialog was closed");
    {
        let store = d.leads().lock().unwrap();
        let pa = store.leads[&f.project.slug]
            .pending_answers
            .first()
            .expect("verbatim answer staged in durable outbox");
        assert!(pa.dialog_closed);
        assert!(!pa.delivered);
        assert!(!pa.board_done);
        assert_eq!(pa.text, "use cockroachdb cluster");
    }

    // Next tick (after 1-min backoff) delivers verbatim text once and completes board + outbox cleanup.
    fake.fail_relay.store(false, Ordering::SeqCst);
    let backoff_due = d.leads().lock().unwrap().leads[&f.project.slug].pending_answers[0]
        .next_attempt_at
        .unwrap();
    d.tick_at(backoff_due).await.unwrap();
    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, m)| {
                t == "Owner answered your question (\"Database?\"): use cockroachdb cluster"
                    && *m == unipi_host::superagent::Delivery::Steer
            })
            .count(),
        1,
        "tick must deliver verbatim text once"
    );
    assert_eq!(
        fake.raw_answers.lock().unwrap().len(),
        1,
        "dialog answered once"
    );
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].pending_answers,
        [] as [unipi_host::superagent_leads::PendingAnswer; 0]
    );
    assert!(f
        .task(&task)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: use cockroachdb cluster"));

    // Case B: multi-question -> dialog closes, steer fails -> owner re-submits with requestId
    // (no E_ALREADY_ANSWERED while outbox entry exists) -> delivers once.
    fake.dialog
        .lock()
        .unwrap()
        .push(unipi_host::superagent_questions::LeadQuestion {
            request_id: "req-n2-retry".into(),
            texts: vec!["Cloud?".into(), "Region?".into()],
            options: vec![vec!["aws".into()], vec!["eu".into(), "us".into()]],
            question_ids: vec!["q0".into(), "q1".into()],
            allow_other: vec![true, true],
        });
    d.tick_at(now + 2_000).await.unwrap();

    fake.fail_relay.store(true, Ordering::SeqCst);
    let err2 = d
        .relay_to_lead_checked(
            &f.project.slug,
            "aws in eu-west-1",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead),
            Some("req-n2-retry"),
        )
        .await
        .unwrap_err();
    assert_eq!(err2.code, "E_RELAY_DELIVERY");

    fake.fail_relay.store(false, Ordering::SeqCst);
    let retry_reply = d
        .relay_to_lead_checked(
            &f.project.slug,
            "aws in eu-west-1",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead),
            Some("req-n2-retry"),
        )
        .await
        .expect("owner retry must resume outbox instead of returning E_ALREADY_ANSWERED");
    assert_eq!(retry_reply["delivered"], true);
    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, m)| {
                t == "Owner answered your question (\"1) Cloud? 2) Region?\"): aws in eu-west-1"
                    && *m == unipi_host::superagent::Delivery::Steer
            })
            .count(),
        1,
        "owner retry must deliver verbatim text once"
    );
    assert_eq!(
        fake.raw_answers.lock().unwrap().len(),
        2,
        "second dialog answered only once"
    );

    // Once the outbox entry is cleared, a duplicate for req-n2-retry returns E_ALREADY_ANSWERED.
    let dup = d
        .relay_to_lead_checked(
            &f.project.slug,
            "aws in eu-west-1",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Steer,
            false,
            Some(&lead),
            Some("req-n2-retry"),
        )
        .await
        .unwrap_err();
    assert_eq!(dup.code, "E_ALREADY_ANSWERED");
}

/// N5 (P2): when the runtime is offline at question timeout, the timeout steer
/// stays pending in the outbox, wakes the SAME lead session once wakeable,
/// delivers the steer once, and clears only after delivery.
#[tokio::test]
async fn offline_timeout_keeps_pending_steer_wakes_same_lead_and_clears_after_delivery() {
    let f = Fixture::new();
    let task = f.add("offline-timeout", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-n5", "Which cache TTL?"));

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    // Runtime goes offline AND cannot wake yet at the timeout instant.
    fake.live.lock().unwrap().clear();
    fake.fail_launch.store(true, Ordering::SeqCst);
    d.tick_at(now + 30 * 60_000).await.unwrap();

    // Ticket is parked blocked and dialog resolved, while timeout steer remains pending!
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert!(fake.dialog.lock().unwrap().is_empty());
    {
        let store = d.leads().lock().unwrap();
        let rec = &store.leads[&f.project.slug];
        assert_eq!(rec.session_id, lead);
        let pa = rec
            .pending_answers
            .first()
            .expect("timeout steer must remain pending while runtime is offline");
        assert!(pa.board_done);
        assert!(pa.dialog_closed);
        assert!(!pa.delivered);
        assert!(pa.delivery_text.contains(&task));
    }

    // Runtime becomes wakeable: once 1-min backoff elapses, tick wakes the SAME lead session, delivers steer, and clears!
    fake.fail_launch.store(false, Ordering::SeqCst);
    d.tick_at(now + 30 * 60_000 + 60_000).await.unwrap();
    {
        let store = d.leads().lock().unwrap();
        let rec = &store.leads[&f.project.slug];
        assert_eq!(rec.session_id, lead, "woke the SAME lead session");
        assert!(rec.pending_answers.is_empty(), "cleared after delivery");
        assert!(rec.question.is_none());
    }
    assert!(fake.live.lock().unwrap().contains(&lead));
    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, m)| t.contains("30 min")
                && t.contains(&task)
                && *m == unipi_host::superagent::Delivery::Steer)
            .count(),
        1,
        "timeout steer delivered once after wake"
    );
}

/// B / N3 (P1): `lead: "reuse"` persists the current `LeadRecord` BEFORE launching;
/// on store-save failure nothing is launched/registered and a later reuse reopens
/// the same chat; on launch failure the reused session enters Stalled and a
/// subsequent ordinary start keeps that same session id (never fresh allocation).
#[tokio::test]
#[allow(
    clippy::many_single_char_names,
    reason = "Short fixture and stream endpoint names follow the protocol scenario notation."
)]
async fn reuse_persists_before_launch_and_handles_save_and_launch_failures_without_new_chat() {
    let f = Fixture::new();
    let a = f.add("first", &[]);
    let b = f.add("second", &[]);
    let c = f.add("third", &[]);
    let session_file = f.home.path().join("reused-n3.jsonl");
    std::fs::write(&session_file, b"{}\n").unwrap();

    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &a).await.unwrap();
    *fake.files.lock().unwrap() = [(lead.clone(), session_file.clone())].into_iter().collect();
    d.leads()
        .lock()
        .unwrap()
        .leads
        .get_mut(&f.project.slug)
        .unwrap()
        .session_file = Some(session_file);

    d.stop(&f.project.slug, &a).await.unwrap();
    fake.live.lock().unwrap().clear();
    let launches_before = fake.launches.load(Ordering::SeqCst);

    // 1. Store-save failure BEFORE launch: nothing launched, nothing registered,
    // ticket rolled back to `todo`, and `lead` stays in `retired`.
    let leads_path = f.layout.home.join("superagent-leads.json");
    let saved_bytes = std::fs::read(&leads_path).unwrap();
    std::fs::remove_file(&leads_path).unwrap();
    std::fs::create_dir_all(leads_path.join("blocker")).unwrap();

    let save_err = d
        .start_tasks_with_lead(
            &f.project.slug,
            std::slice::from_ref(&b),
            None,
            unipi_host::superagent_presets::Overrides::default(),
            Some("reuse"),
        )
        .await
        .expect_err("store save failure before reuse launch must return error");
    assert_eq!(save_err.code, "E_SUPERAGENT_STORAGE");
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        launches_before,
        "no runtime launched when pre-launch save failed"
    );
    assert!(!d.is_lead_session(&lead), "not registered in lead_sessions");
    assert!(fake.live.lock().unwrap().is_empty());
    assert_eq!(
        f.task(&b).status,
        Status::Todo,
        "reservation rolled back cleanly"
    );

    // Restore storage, then reuse with launch failure: enters Stalled on `lead`!
    std::fs::remove_dir_all(&leads_path).unwrap();
    std::fs::write(&leads_path, &saved_bytes).unwrap();
    fake.fail_launch.store(true, Ordering::SeqCst);
    let launch_err = d
        .start_tasks_with_lead(
            &f.project.slug,
            std::slice::from_ref(&b),
            None,
            unipi_host::superagent_presets::Overrides::default(),
            Some("reuse"),
        )
        .await
        .expect_err("launch failure on reuse must return E_WORKER_LAUNCH");
    assert_eq!(launch_err.code, "E_WORKER_LAUNCH");
    {
        let store = d.leads().lock().unwrap();
        let rec = store
            .leads
            .get(&f.project.slug)
            .expect("reused session persisted as current record before launch");
        assert_eq!(rec.session_id, lead);
        assert_eq!(rec.state, unipi_host::superagent_leads::LeadState::Stalled);
    }

    // 2. Subsequent ordinary `start` (without `reuse`) wakes the SAME `lead` chat — never mints a new id!
    fake.fail_launch.store(false, Ordering::SeqCst);
    let started = d.start(&f.project.slug, &c).await.unwrap();
    assert_eq!(
        started, lead,
        "later start must wake the same reused session id"
    );
}

/// D1: an open `ask_user` dialog with NO `in_progress` ticket attributed still
/// counts as `Busy` before timeout, then at timeout resolves as skipped + steers
/// `'The owner did not answer within N min; carry on without it or put the question on a ticket.'`
/// with NO ticket move.
#[tokio::test]
async fn unattributed_open_question_counts_as_busy_then_times_out_with_skipped_and_steer_without_ticket_move(
) {
    let f = Fixture::new();
    let task = f.add("already-finished", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();

    // Move `task` out of `in_progress` so the lead holds NO `in_progress` ticket,
    // while keeping an open `ask_user` dialog on the live lead.
    commands::finish(
        &f.layout,
        f.project.clone(),
        ChainGate::InReview,
        &task,
        &lead,
        "done",
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    assert_eq!(f.task(&task).status, Status::InReview);

    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog.lock().unwrap().push(lead_question(
        "req-unattr",
        "Should I refactor anything else?",
    ));

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    // Counts as Busy while open (does not transition to Idle or emit a ticketless push).
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].state,
        unipi_host::superagent_leads::LeadState::Working
    );
    assert_eq!(fake.notices.load(Ordering::SeqCst), 0);
    assert_eq!(
        d.status().await.unwrap()["needsYou"]
            .as_array()
            .unwrap()
            .as_slice(),
        [] as [serde_json::Value; 0]
    );

    // At timeout: resolved as skipped + unattributed timeout steer, no ticket move!
    d.tick_at(now + 30 * 60_000).await.unwrap();
    assert_eq!(f.task(&task).status, Status::InReview, "no ticket move");
    assert!(fake.dialog.lock().unwrap().is_empty(), "dialog resolved");
    assert_eq!(
        fake.answers.lock().unwrap().last().unwrap(),
        &("req-unattr".to_owned(), "<skipped>".to_owned())
    );
    assert_eq!(
        fake.deliveries.lock().unwrap().last().unwrap(),
        &(
            "The owner did not answer within 30 min; carry on without it or put the question on a ticket.".to_owned(),
            unipi_host::superagent::Delivery::Steer,
        )
    );
    assert!(d.leads().lock().unwrap().leads[&f.project.slug]
        .question
        .is_none());
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].pending_answers,
        [] as [unipi_host::superagent_leads::PendingAnswer; 0]
    );
}

/// D2: timeout completion and outbox steps run even when the lead is `Stalled`.
#[tokio::test]
async fn stalled_lead_completes_timeout_and_outbox_steps_regardless_of_state() {
    let f = Fixture::new();
    let task = f.add("stalled-timeout", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-stalled", "Which port?"));

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    // Force the lead into `Stalled` before the question timeout arrives.
    unipi_host::superagent_leads::enter_stall(
        &d,
        &f.project,
        unipi_host::superagent_leads::HealthKind::Model,
        "429 rate limit".into(),
        now + 1_000,
        Some(now + 60 * 60_000),
    )
    .await
    .unwrap();
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].state,
        unipi_host::superagent_leads::LeadState::Stalled
    );

    // At 30 min (still before the 60 min stall probe!), timeout completion STILL runs:
    // parks the ticket blocked, resolves the dialog as skipped, and steers the timeout text.
    d.tick_at(now + 30 * 60_000).await.unwrap();
    assert_eq!(f.task(&task).status, Status::Blocked);
    assert_eq!(f.task(&task).run.as_ref().unwrap().session, lead);
    assert!(fake.dialog.lock().unwrap().is_empty());
    let store = d.leads().lock().unwrap();
    let rec = &store.leads[&f.project.slug];
    assert_eq!(rec.state, unipi_host::superagent_leads::LeadState::Stalled);
    assert!(rec.question.is_none());
    assert_eq!(
        rec.pending_answers,
        [] as [unipi_host::superagent_leads::PendingAnswer; 0]
    );
}

/// F1 (P1): (1) selected option + dialog response fails once -> next tick submits
/// the SAME selected option (never `<skipped>`); (2) allowed free text + dialog
/// fails once and then dialog disappears -> steer fallback
/// `'Owner answered your question ("<q>"): <verbatim>'` delivered once, then cleared.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn f1_selected_option_and_free_text_dialog_recovery_preserve_owner_words() {
    let f = Fixture::new();
    let task = f.add("f1-dialog-recovery", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());

    // Case 1: Selected option ("postgres"), dialog fails once -> next tick submits "postgres".
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-f1-opt", "Pick engine?"));
    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();

    fake.fail_answer_once.store(true, Ordering::SeqCst);
    let err = d
        .relay_to_lead_checked(
            &f.project.slug,
            "postgres",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Queue,
            false,
            Some(&lead),
            Some("req-f1-opt"),
        )
        .await
        .expect_err("initial dialog failure must surface");
    assert_eq!(err.code, "E_ALREADY_ANSWERED");
    {
        let store = d.leads().lock().unwrap();
        let pa = &store.leads[&f.project.slug].pending_answers[0];
        assert!(!pa.dialog_closed);
        assert!(
            !pa.delivered,
            "must NOT mark delivered before dialog succeeds"
        );
        assert!(
            pa.dialog_answer.is_some(),
            "persists exact encoded dialog_answer"
        );
    }

    let due1 = d.leads().lock().unwrap().leads[&f.project.slug].pending_answers[0]
        .next_attempt_at
        .unwrap();
    d.tick_at(due1).await.unwrap();
    assert_eq!(
        fake.answers.lock().unwrap().last().unwrap(),
        &("req-f1-opt".to_owned(), "postgres".to_owned()),
        "recovery submits the exact selected option, NOT <skipped>"
    );
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].pending_answers,
        [] as [unipi_host::superagent_leads::PendingAnswer; 0]
    );
    assert!(f
        .task(&task)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: postgres"));

    // Case 2: Allowed free text ("useduckdb"), dialog fails once, then dialog disappears
    // before tick -> steer fallback `'Owner answered your question ("<q>"): <verbatim>'` delivered once.
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-f1-free", "Any notes?"));
    let t2 = due1 + 5_000;
    d.tick_at(t2).await.unwrap();

    fake.fail_answer_once.store(true, Ordering::SeqCst);
    let err2 = d
        .relay_to_lead_checked(
            &f.project.slug,
            "use custom duckdb build",
            std::slice::from_ref(&task),
            unipi_host::superagent::Delivery::Queue,
            false,
            Some(&lead),
            Some("req-f1-free"),
        )
        .await
        .expect_err("initial dialog failure must surface");
    assert_eq!(err2.code, "E_ALREADY_ANSWERED");

    // Dialog request vanishes before recovery tick!
    fake.dialog.lock().unwrap().clear();
    let due2 = d.leads().lock().unwrap().leads[&f.project.slug].pending_answers[0]
        .next_attempt_at
        .unwrap();
    d.tick_at(due2).await.unwrap();

    assert_eq!(
        fake.deliveries
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, m)| {
                t == "Owner answered your question (\"Any notes?\"): use custom duckdb build"
                    && *m == unipi_host::superagent::Delivery::Steer
            })
            .count(),
        1,
        "steer fallback delivered once when dialog request disappeared"
    );
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].pending_answers,
        [] as [unipi_host::superagent_leads::PendingAnswer; 0]
    );
    assert!(f
        .task(&task)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: use custom duckdb build"));
}

/// F2 (P1): per-operation `pending_answers: Vec<PendingAnswer>` isolates tickets so
/// ticket A's failing board resume stays pending while ticket B is delivered and
/// resumed to `in_progress`, and a terminal board-resume conflict records a System
/// note and drops the entry without failing.
#[tokio::test]
async fn f2_per_operation_outbox_isolates_tickets_and_resolves_board_conflict() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let task_a = f.add("ticket-a", &[]);
    let task_b = f.add("ticket-b", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task_a).await.unwrap();
    d.start(&f.project.slug, &task_b).await.unwrap();

    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task_a,
        &lead,
        Some("Question A?"),
    )
    .unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task_b,
        &lead,
        Some("Question B?"),
    )
    .unwrap();

    // 1. Make board unavailable when answering ticket A: A delivers to lead, board resume fails.
    let tasks_dir = f.layout.tasks_dir(&f.project.slug);
    let orig = std::fs::metadata(&tasks_dir).unwrap().permissions();
    let mut ro = orig.clone();
    ro.set_mode(0o555);
    std::fs::set_permissions(&tasks_dir, ro).unwrap();
    let err_a = d
        .answer_blocked_question(&lead, &task_a, "answer-for-a")
        .await
        .expect_err("ticket A board write failure surfaces E_BOARD_RESUME_FAILED");
    std::fs::set_permissions(&tasks_dir, orig).unwrap();
    assert_eq!(err_a.code, "E_BOARD_RESUME_FAILED");

    // Break ONLY ticket A's task file so A's board resume remains unavailable!
    let task_a_path = tasks_dir.join(format!("{task_a}.md"));
    let task_a_backup = std::fs::read(&task_a_path).unwrap();
    std::fs::write(&task_a_path, b"---\n: [corrupt-yaml\n---\n").unwrap();

    // 2. Owner answers ticket B: must NOT touch ticket A's failing entry; B delivers & resumes!
    let ok_b = d
        .answer_blocked_question(&lead, &task_b, "answer-for-b")
        .await
        .expect("ticket B must succeed despite ticket A's broken board entry");
    assert_eq!(ok_b["delivered"], true);
    assert_eq!(f.task(&task_b).status, Status::InProgress);
    assert!(f
        .task(&task_b)
        .activity
        .last()
        .unwrap()
        .text
        .contains("Owner answer: answer-for-b"));
    {
        let store = d.leads().lock().unwrap();
        let pending = &store.leads[&f.project.slug].pending_answers;
        assert_eq!(
            pending.len(),
            1,
            "ticket A remains pending while ticket B cleared"
        );
        assert_eq!(pending[0].task_id, task_a);
        assert!(pending[0].delivered && !pending[0].board_done);
    }

    // 3. Restore ticket A's file, then create a terminal CONFLICT (move A to in_review).
    std::fs::write(&task_a_path, &task_a_backup).unwrap();
    {
        let board = unipi_kanboard::board::Board::open(&f.layout, f.project.clone()).unwrap();
        let mut ta = board.get(&task_a).unwrap();
        ta.status = Status::InReview;
        board.save(&ta).unwrap();
    }
    let due_a = d.leads().lock().unwrap().leads[&f.project.slug].pending_answers[0]
        .next_attempt_at
        .unwrap();
    d.tick_at(due_a + 1_000).await.unwrap();

    assert!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .pending_answers
            .is_empty(),
        "conflict is terminal: outbox entry dropped"
    );
    let ta = f.task(&task_a);
    assert!(
        ta.activity.iter().any(|a| {
            a.actor == Actor::System
                && a.text
                    .starts_with("Owner answer was delivered to the lead; board resume skipped:")
        }),
        "conflict records System ticket note"
    );
}

/// F3 (P2): sustained launch failure honours exponential backoff (no wake on every tick),
/// enters `Stalled` after 3 consecutive failed runtime steps with ONE notice and a health
/// card, honours stall probe deadlines thereafter, and explicit owner Retry bypasses backoff.
#[tokio::test]
async fn f3_outbox_exponential_backoff_enters_stall_once_and_explicit_retry_bypasses() {
    let f = Fixture::new();
    let task = f.add("f3-backoff", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-f3", "Deploy target?"));

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now).await.unwrap();
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1, "1 question notice");

    // Unload runtime and make launches fail continuously.
    fake.live.lock().unwrap().remove(&lead);
    fake.fail_launch.store(true, Ordering::SeqCst);
    let base_launches = fake.launches.load(Ordering::SeqCst);

    // Timeout at t0 = now + 30m: attempt #1 fails -> next_attempt_at = t0 + 1m, not stalled yet.
    let t0 = now + 30 * 60_000;
    d.tick_at(t0).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), base_launches + 1);
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        1,
        "not stalled yet on 1st failure"
    );

    // Tick 10s later: NO wake attempt!
    d.tick_at(t0 + 10_000).await.unwrap();
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        base_launches + 1,
        "honours 1m backoff"
    );

    // Tick at t0 + 1m: attempt #2 fails -> next_attempt_at = (t0 + 1m) + 2m.
    d.tick_at(t0 + 60_000).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), base_launches + 2);
    assert_eq!(fake.notices.load(Ordering::SeqCst), 1);

    // Tick 1m later (before 2m backoff): NO wake attempt!
    d.tick_at(t0 + 120_000).await.unwrap();
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        base_launches + 2,
        "honours 2m backoff"
    );

    // Tick at t0 + 3m: attempt #3 fails -> enters Stalled on SAME session, ONE stall notice!
    let t3 = t0 + 180_000;
    d.tick_at(t3).await.unwrap();
    assert_eq!(fake.launches.load(Ordering::SeqCst), base_launches + 3);
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        2,
        "1 question + 1 stall notice"
    );
    {
        let store = d.leads().lock().unwrap();
        let rec = &store.leads[&f.project.slug];
        assert_eq!(rec.session_id, lead);
        assert_eq!(rec.state, unipi_host::superagent_leads::LeadState::Stalled);
        assert!(rec.health.is_some(), "health card present");
    }

    // Even after 5m (past 4m entry backoff, before 30m stall probe): honours stall probe deadline!
    d.tick_at(t3 + 5 * 60_000).await.unwrap();
    assert_eq!(
        fake.launches.load(Ordering::SeqCst),
        base_launches + 3,
        "honours stall probe deadline once stalled"
    );
    assert_eq!(
        fake.notices.load(Ordering::SeqCst),
        2,
        "still one stall notice"
    );

    // Explicit owner Retry on the health card bypasses backoff + probe deadline immediately!
    fake.fail_launch.store(false, Ordering::SeqCst);
    unipi_host::superagent_leads::lead_action(&d, &lead, "retry", None)
        .await
        .unwrap();
    assert!(
        d.leads().lock().unwrap().leads[&f.project.slug]
            .pending_answers
            .is_empty(),
        "explicit Retry flushes outbox immediately"
    );
}

/// F4 (P2): bookkeeping-only outbox entries (`delivered = true, board_done = false`)
/// do NOT suppress warm-unload once `warmMinutes` elapses.
#[tokio::test]
async fn f4_bookkeeping_only_outbox_entry_does_not_suppress_warm_unload() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let task = f.add("f4-warm-unload", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let lead = d.start(&f.project.slug, &task).await.unwrap();

    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &task,
        &lead,
        Some("Cache TTL?"),
    )
    .unwrap();

    // Deliver answer while board write fails -> entry is delivered=true, board_done=false.
    let tasks_dir = f.layout.tasks_dir(&f.project.slug);
    let orig = std::fs::metadata(&tasks_dir).unwrap().permissions();
    let mut ro = orig.clone();
    ro.set_mode(0o555);
    std::fs::set_permissions(&tasks_dir, ro).unwrap();
    let _ = d.answer_blocked_question(&lead, &task, "300s").await;
    std::fs::set_permissions(&tasks_dir, orig).unwrap();

    // Keep task file unreadable so board_done stays false on tick, and mark turn ended -> Waiting.
    let task_path = tasks_dir.join(format!("{task}.md"));
    let backup = std::fs::read(&task_path).unwrap();
    *fake.outcome.lock().unwrap() = Some(String::new());
    let t0 = unipi_host::superagent_leads::now_ms();
    // Put lead into Waiting with `last_active_at = t0` while task is still readable,
    // then break only `task_path`? Wait: `observe` calls `held_tickets` which reads all tasks,
    // so keep `tasks_dir` readable (`0o555` blocks `board.save` while allowing `board.tasks`!).
    let mut ro2 = std::fs::metadata(&tasks_dir).unwrap().permissions();
    ro2.set_mode(0o555);
    std::fs::set_permissions(&tasks_dir, ro2).unwrap();

    d.tick_at(t0).await.unwrap();
    assert_eq!(
        d.leads().lock().unwrap().leads[&f.project.slug].state,
        unipi_host::superagent_leads::LeadState::Waiting
    );

    // Advance past default warm_minutes (30 min): runtime MUST unload despite bookkeeping entry!
    d.tick_at(t0 + 31 * 60_000).await.unwrap();
    std::fs::set_permissions(&tasks_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&task_path, backup).unwrap();

    let store = d.leads().lock().unwrap();
    let rec = &store.leads[&f.project.slug];
    assert_eq!(rec.state, unipi_host::superagent_leads::LeadState::Unloaded);
    assert!(!fake.live.lock().unwrap().contains(&lead));
    assert_eq!(
        rec.pending_answers.len(),
        1,
        "bookkeeping entry preserved across warm unload"
    );
    assert!(rec.pending_answers[0].delivered && !rec.pending_answers[0].board_done);
}

/// Tick isolation: an I/O or parse error on project A's board during `lead_tick`
/// (`flush_pending_answers` / `question_step`) is logged and skipped without
/// aborting `lead_tick` for project B.
#[tokio::test]
async fn tick_isolation_skips_lead_with_broken_board_and_advances_healthy_lead() {
    let f = Fixture::new();
    let root_b = tempfile::tempdir().unwrap();
    let proj_b = Project::create(&f.layout, root_b.path(), Some("beta"), Some("BETA")).unwrap();
    let task_a = f.add("alpha-task", &[]);
    let task_b: String = commands::add(
        &f.layout,
        proj_b.clone(),
        &Common::new(Actor::User, ChainGate::InReview),
        "beta-task",
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
        .into();

    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;
    let _lead_a = d.start(&f.project.slug, &task_a).await.unwrap();
    let lead_b = d.start(&proj_b.slug, &task_b).await.unwrap();

    *fake.outcome.lock().unwrap() = Some(String::new());
    fake.dialog
        .lock()
        .unwrap()
        .push(lead_question("req-beta", &format!("{task_b}: Pick mode?")));

    // Corrupt project A's task file so `held_tickets` in `question_step` errors on project A!
    let task_a_path = f
        .layout
        .tasks_dir(&f.project.slug)
        .join(format!("{task_a}.md"));
    let backup_a = std::fs::read(&task_a_path).unwrap();
    std::fs::write(&task_a_path, b"---\n: [broken\n---\n").unwrap();

    let now = unipi_host::superagent_leads::now_ms();
    d.tick_at(now)
        .await
        .expect("lead_tick must isolate project A's error");

    std::fs::write(&task_a_path, backup_a).unwrap();

    let store = d.leads().lock().unwrap();
    let rec_b = &store.leads[&proj_b.slug];
    assert_eq!(rec_b.session_id, lead_b);
    assert_eq!(
        rec_b.question.as_ref().map(|q| q.request_id.as_str()),
        Some("req-beta"),
        "project B's question_step ran to completion despite project A's error"
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete scenario and resource lifecycle together."
)]
async fn retire_lead_moves_in_progress_and_blocked_tickets_to_backlog_with_incomplete_feed_and_brief(
) {
    let f = Fixture::new();
    let in_prog = f.add("unfinished-work", &[]);
    let blocked_q = f.add("blocked-on-question", &[]);
    let fake = Arc::new(Fake::default());
    let d = f.dispatcher(fake.clone()).await;

    let lead_1 = d
        .start_tasks(
            &f.project.slug,
            &[in_prog.clone(), blocked_q.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &blocked_q,
        &lead_1,
        Some("Which cache backend should we wire?"),
    )
    .unwrap();
    let in_prog_before_len = f.task(&in_prog).activity.len();
    let blocked_before_len = f.task(&blocked_q).activity.len();

    assert!(d.stop(&f.project.slug, &in_prog).await.unwrap());

    let stored_in_prog = f.task(&in_prog);
    assert_eq!(stored_in_prog.status, Status::Backlog);
    assert!(stored_in_prog.run.is_none());
    assert_eq!(
        stored_in_prog.activity.len(),
        in_prog_before_len + 1,
        "at most one activity line added on retirement"
    );
    let in_prog_incomplete: Vec<_> = stored_in_prog
        .activity
        .iter()
        .filter(|a| {
            a.text
                .starts_with(unipi_kanboard::dispatcher::RELEASED_INCOMPLETE_PREFIX)
        })
        .collect();
    assert_eq!(in_prog_incomplete.len(), 1);
    assert_eq!(in_prog_incomplete[0].actor, Actor::System);
    assert!(in_prog_incomplete[0].text.contains("was in_progress"));
    assert!(in_prog_incomplete[0]
        .text
        .contains(&format!("held by session {lead_1}")));
    assert!(in_prog_incomplete[0].text.contains("lead stopped"));
    assert!(!in_prog_incomplete[0].text.contains("Open question:"));

    let stored_blocked = f.task(&blocked_q);
    assert_eq!(stored_blocked.status, Status::Backlog);
    assert!(stored_blocked.run.is_none());
    assert_eq!(
        stored_blocked.activity.len(),
        blocked_before_len + 1,
        "at most one activity line added on retirement"
    );
    let blocked_incomplete: Vec<_> = stored_blocked
        .activity
        .iter()
        .filter(|a| {
            a.text
                .starts_with(unipi_kanboard::dispatcher::RELEASED_INCOMPLETE_PREFIX)
        })
        .collect();
    assert_eq!(blocked_incomplete.len(), 1);
    assert_eq!(blocked_incomplete[0].actor, Actor::System);
    assert!(blocked_incomplete[0].text.contains("was blocked"));
    assert!(blocked_incomplete[0]
        .text
        .contains(&format!("held by session {lead_1}")));
    assert!(blocked_incomplete[0].text.contains("lead stopped"));
    assert!(
        blocked_incomplete[0]
            .text
            .contains("Open question: Which cache backend should we wire?. "),
        "{}",
        blocked_incomplete[0].text
    );
    let expected_blocked_line = blocked_incomplete[0].text.clone();

    // Starting a new lead on the released tickets delivers their `Released incomplete:`
    // lines verbatim in the brief, and archiving that lead records `lead chat archived`.
    fake.deliveries.lock().unwrap().clear();
    let lead_2 = d
        .start_tasks(
            &f.project.slug,
            &[in_prog.clone(), blocked_q.clone()],
            None,
            unipi_host::superagent_presets::Overrides::default(),
        )
        .await
        .unwrap();
    assert_ne!(lead_1, lead_2);
    // Re-read the brief via a queued task or check that `lead_brief` carries the line verbatim.
    let preset = d.leads().lock().unwrap().leads[&f.project.slug]
        .preset
        .clone();
    let brief = unipi_kanboard::board::Board::open(&f.layout, f.project.clone())
        .map(|board| vec![board.get(&in_prog).unwrap(), board.get(&blocked_q).unwrap()])
        .unwrap();
    assert!(brief[1]
        .activity
        .iter()
        .any(|a| a.text == expected_blocked_line));
    // Queue another ticket into live `lead_2` after parking `blocked_q` again, then archive `lead_2`.
    unipi_kanboard::dispatcher::approval_transition(
        &f.layout,
        f.project.clone(),
        &blocked_q,
        &lead_2,
        Some("Second question before archive?"),
    )
    .unwrap();
    let _ = preset;
    d.sessions_archived(std::slice::from_ref(&lead_2)).await;
    let after_archive = f.task(&blocked_q);
    assert_eq!(after_archive.status, Status::Backlog);
    assert!(after_archive.run.is_none());
    let latest_line = after_archive.activity.last().unwrap();
    assert!(latest_line
        .text
        .starts_with(unipi_kanboard::dispatcher::RELEASED_INCOMPLETE_PREFIX));
    assert!(latest_line
        .text
        .contains(&format!("held by session {lead_2}")));
    assert!(latest_line.text.contains("lead chat archived"));
    assert!(latest_line
        .text
        .contains("Open question: Second question before archive?. "));

    // Delivering into a live lead includes the LATEST `Released incomplete:` line verbatim in the brief.
    let holder = f.add("holder", &[]);
    let lead_3 = d.start(&f.project.slug, &holder).await.unwrap();
    fake.deliveries.lock().unwrap().clear();
    d.start(&f.project.slug, &blocked_q).await.unwrap();
    let delivered_brief = fake.deliveries.lock().unwrap()[0].0.clone();
    assert!(
        delivered_brief.contains(&latest_line.text),
        "live relay brief must contain latest Released incomplete line verbatim:\n{delivered_brief}"
    );
    assert!(
        !delivered_brief.contains(&expected_blocked_line),
        "older Released incomplete line must be superseded by the latest:\n{delivered_brief}"
    );
    let _ = lead_3;
}
