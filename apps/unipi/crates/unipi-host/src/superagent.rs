//! Machine-wide Superagent dispatcher. The kernel lock covers the entire owner lifetime;
//! the async transaction lock covers watcher/manual reservation through session launch.
use futures_util::future::BoxFuture;
use oino_core::{
    runtime::{AgentSessionRuntime, SessionOptions, SessionSource},
    session::{InputSource, PromptOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;
use unipi_kanboard::{
    Layout, Project,
    board::Board,
    dispatcher,
    model::{RunOwner, Status, Task},
};

pub const INTERRUPTED: &str = "Worker was interrupted; move to todo to retry";

/// Stable machine-readable refusal and actionable explanation.
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct DispatchError {
    pub code: &'static str,
    pub message: String,
}
type Result<T> = std::result::Result<T, DispatchError>;
fn fail(code: &'static str, message: impl ToString) -> DispatchError {
    DispatchError {
        code,
        message: message.to_string(),
    }
}
fn storage(e: impl ToString) -> DispatchError {
    fail("E_SUPERAGENT_STORAGE", e)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub concurrency_cap: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            concurrency_cap: 1,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !(1..=3).contains(&self.concurrency_cap) {
            return Err(fail(
                "E_SCHEMA",
                "concurrencyCap must be an integer between 1 and 3",
            ));
        }
        Ok(())
    }
}

/// Runtime boundary used by production and deterministic crash/concurrency tests.
pub trait Workers: Send + Sync {
    fn live<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool>;
    /// Must create with exactly this durable id, or attach to that id if it exists.
    fn launch<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>>;
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, std::result::Result<(), String>>;
    fn relay<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>>;
    /// None while busy, Some(last message) once the worker ends without a transition.
    fn ended<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>>;
    fn retire<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn approvals<'a>(
        &'a self,
        _id: &'a str,
    ) -> BoxFuture<'a, Vec<oino_core::session::permission::ApprovalRequest>> {
        Box::pin(async { Vec::new() })
    }
    fn approval_response<'a>(
        &'a self,
        _id: &'a str,
        _response: oino_core::session::permission::ApprovalResponse,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Err("Worker is not live".into()) })
    }
    fn progress<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Value> {
        Box::pin(async { json!({"step":null,"tool":null,"cost":null}) })
    }
}

pub struct Dispatcher {
    _owner: File,
    layout: Layout,
    config_path: PathBuf,
    transaction: Mutex<Config>,
    workers: Arc<dyn Workers>,
    active: std::sync::Mutex<HashMap<String, String>>,
}
impl Dispatcher {
    /// Reconcile all local durable reservations before accepting any dispatch.
    pub async fn acquire(
        layout: Layout,
        config_path: PathBuf,
        workers: Arc<dyn Workers>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&layout.home).map_err(storage)?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(layout.home.join("dispatcher.lock"))
            .map_err(storage)?;
        owner.try_lock().map_err(|_| {
            fail(
                "E_DISPATCHER_OWNER",
                "Another host owns dispatch; use that host or stop it first",
            )
        })?;
        let config = match std::fs::read(&config_path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(storage)?
                .get("superagent")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(storage)?
                .unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(storage(e)),
        };
        config.validate()?;
        let this = Arc::new(Self {
            _owner: owner,
            layout,
            config_path,
            transaction: Mutex::new(config),
            workers,
            active: std::sync::Mutex::new(HashMap::new()),
        });
        for (project, task) in this.claims()? {
            let run = task.run.as_ref().ok_or_else(|| storage("Missing run"))?;
            if run.host == unipi_kanboard::commands::hostname()
                && !this.workers.live(&run.session).await
            {
                dispatcher::block(&this.layout, project, &task.id, &run.session, INTERRUPTED)
                    .map_err(storage)?;
            } else {
                this.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(run.session.clone(), project.slug);
            }
        }
        Ok(this)
    }
    fn claims(&self) -> Result<Vec<(Project, Task)>> {
        let mut out = Vec::new();
        for project in self.layout.list_projects().map_err(storage)? {
            for task in Board::open(&self.layout, project.clone())
                .map_err(storage)?
                .tasks()
                .map_err(storage)?
            {
                if matches!(task.status, Status::InProgress | Status::Blocked)
                    && task
                        .run
                        .as_ref()
                        .is_some_and(|r| r.owner == RunOwner::Dispatcher)
                {
                    out.push((project.clone(), task));
                }
            }
        }
        Ok(out)
    }
    pub async fn configure(&self, config: Config) -> Result<Config> {
        config.validate()?;
        let mut current = self.transaction.lock().await;
        let mut value = match std::fs::read(&self.config_path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(storage)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(e) => return Err(storage(e)),
        };
        value["superagent"] = serde_json::to_value(&config).map_err(storage)?;
        crate::data_dir::write_json(&self.config_path, &value).map_err(storage)?;
        *current = config.clone();
        Ok(config)
    }
    /// Shared watcher/manual transaction. Reservation is durable before runtime creation.
    pub async fn start(&self, project_id: &str, task_id: &str) -> Result<String> {
        self.start_transaction(project_id, task_id, false).await
    }
    async fn start_transaction(
        &self,
        project_id: &str,
        task_id: &str,
        watcher: bool,
    ) -> Result<String> {
        let config = self.transaction.lock().await;
        if watcher && !config.enabled {
            return Err(fail(
                "E_WATCHER_OFF",
                "Watcher is disabled; manually start a ticket or enable it",
            ));
        }
        let claims = self.claims()?;
        let mut occupied: HashMap<String, String> = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for (project, task) in claims {
            if let Some(run) = task.run {
                occupied.insert(run.session, project.slug);
            }
        }
        if occupied.values().any(|p| p == project_id) {
            return Err(fail(
                "E_PROJECT_WORKER",
                "Project has an active worker; wait for it to end or stop it",
            ));
        }
        if occupied.len() >= config.concurrency_cap {
            return Err(fail(
                "E_WORKER_CAP",
                "Worker cap reached; wait for a worker or raise concurrencyCap",
            ));
        }
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        let id = uuid::Uuid::now_v7().to_string();
        let task = dispatcher::reserve(&self.layout, project.clone(), task_id, &id)
            .map_err(|e| fail("E_WORKER_REFUSED", e))?;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), project_id.into());
        if let Err(e) = self.workers.launch(&project, &task, &id).await {
            dispatcher::block(
                &self.layout,
                project,
                task_id,
                &id,
                &format!("Worker could not start: {e}; move to todo to retry"),
            )
            .map_err(storage)?;
            self.active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            return Err(fail("E_WORKER_LAUNCH", e));
        }
        Ok(id)
    }
    /// Reap completed workers, then offer ready todo tickets through start's transaction.
    pub async fn tick(&self) -> Result<()> {
        {
            let _transaction = self.transaction.lock().await;
            let active: Vec<String> = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .keys()
                .cloned()
                .collect();
            for (project, task) in self.claims()? {
                let run = task.run.as_ref().ok_or_else(|| storage("Missing run"))?;
                if run.host != unipi_kanboard::commands::hostname() {
                    continue;
                }
                if let Some(last) = self.workers.ended(&run.session).await {
                    let question = if last.trim().is_empty() {
                        "Worker ended without a transition; move to todo to retry"
                    } else {
                        &last
                    };
                    dispatcher::block(&self.layout, project, &task.id, &run.session, question)
                        .map_err(storage)?;
                }
            }
            for id in active {
                if self.workers.ended(&id).await.is_some() {
                    self.active
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&id);
                    self.workers.retire(&id).await;
                }
            }
        }
        if !self.transaction.lock().await.enabled {
            return Ok(());
        }
        for project in self.layout.list_projects().map_err(storage)? {
            if project.archived {
                continue;
            }
            for task in Board::open(&self.layout, project.clone())
                .map_err(storage)?
                .tasks()
                .map_err(storage)?
            {
                if task.status == Status::Todo {
                    let _ = self.start_transaction(&project.slug, &task.id, true).await;
                }
            }
        }
        Ok(())
    }
    pub async fn stop(&self, project_id: &str, task_id: &str) -> Result<bool> {
        let _transaction = self.transaction.lock().await;
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        let task = Board::open(&self.layout, project.clone())
            .map_err(storage)?
            .get(task_id)
            .map_err(storage)?;
        let Some(run) = task
            .run
            .as_ref()
            .filter(|r| dispatcher::matches_claim(&task, &r.session))
        else {
            return Ok(false);
        };
        self.workers
            .stop(&run.session)
            .await
            .map_err(|e| fail("E_WORKER_STOP", e))?;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&run.session);
        dispatcher::block(
            &self.layout,
            project,
            task_id,
            &run.session,
            "Worker stopped; move to todo to retry",
        )
        .map_err(storage)
    }
    pub async fn relay(&self, project_id: &str, task_id: &str, text: &str) -> Result<Value> {
        let _transaction = self.transaction.lock().await;
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        let task = Board::open(&self.layout, project.clone())
            .map_err(storage)?
            .get(task_id)
            .map_err(storage)?;
        let run = task
            .run
            .as_ref()
            .filter(|r| dispatcher::matches_claim(&task, &r.session))
            .ok_or_else(|| {
                fail(
                    "E_WORKER_CLAIM",
                    "Ticket has no dispatcher worker; refresh the board",
                )
            })?;
        if !self.workers.live(&run.session).await
            || self.workers.ended(&run.session).await.is_some()
        {
            return Err(fail(
                "E_WORKER_NOT_LIVE",
                "Worker has ended; refresh the board before relaying",
            ));
        }
        let comment = uuid::Uuid::now_v7().to_string();
        let mut reply =
            dispatcher::relay(&self.layout, project, task_id, &run.session, text, &comment)
                .map_err(storage)?;
        match self.workers.relay(&run.session, text).await {
            Ok(()) => reply["delivered"] = json!(true),
            Err(e) => {
                reply["delivered"] = json!(false);
                reply["error"] = json!({"code":"E_RELAY_DELIVERY", "message":format!("Comment saved, but delivery failed: {e}")});
            }
        }
        Ok(reply)
    }
    pub async fn approval_response(
        &self,
        project_id: &str,
        task_id: &str,
        session_id: &str,
        response: oino_core::session::permission::ApprovalResponse,
    ) -> Result<()> {
        let _transaction = self.transaction.lock().await;
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        let task = Board::open(&self.layout, project)
            .map_err(storage)?
            .get(task_id)
            .map_err(storage)?;
        let owned = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .is_some_and(|p| p == project_id);
        if !owned
            || !task.run.as_ref().is_some_and(|r| {
                r.session == session_id && dispatcher::matches_claim(&task, session_id)
            })
            || !self.workers.live(session_id).await
            || self.workers.ended(session_id).await.is_some()
        {
            return Err(fail(
                "E_WORKER_CLAIM",
                "Ticket has no live dispatcher worker",
            ));
        }
        self.workers
            .approval_response(session_id, response)
            .await
            .map_err(|e| fail("E_APPROVAL", e))
    }
    pub async fn status(&self) -> Result<Value> {
        let config = self.transaction.lock().await;
        let mut workers = Vec::new();
        let mut needs_you = Vec::new();
        for project in self.layout.list_projects().map_err(storage)? {
            for task in Board::open(&self.layout, project.clone())
                .map_err(storage)?
                .tasks()
                .map_err(storage)?
            {
                if let Some(run) = &task.run {
                    if run.owner == RunOwner::Dispatcher {
                        let progress = self.workers.progress(&run.session).await;
                        workers.push(json!({"projectId":project.slug,"taskId":task.id,"sessionId":run.session,
                            "autonomy":dispatcher::autonomy(&task),"step":progress["step"],"tool":progress["tool"],"startedAt":run.started,"cost":progress["cost"]}));
                        if dispatcher::matches_claim(&task, &run.session)
                            && self.workers.live(&run.session).await
                            && self.workers.ended(&run.session).await.is_none()
                        {
                            for approval in self.workers.approvals(&run.session).await {
                                needs_you.push(json!({"projectId":project.slug,"taskId":task.id,"status":"blocked", "workerSessionId":run.session, "requestId":approval.request_id, "sessionId":approval.session_id, "tool":approval.tool, "command":approval.arguments.get("command"), "summary":approval.arguments.get("summary"), "risk":approval.risk_level, "ceiling":approval.ceiling, "reason":approval.reason}));
                            }
                        }
                    }
                }
                if matches!(task.status, Status::Blocked | Status::InReview) && task.run.is_none() {
                    needs_you.push(json!({"projectId":project.slug,"taskId":task.id,"status":task.status,
                        "question":task.activity.iter().rev().find(|a| a.text.starts_with("blocked:")).map(|a| a.text.trim_start_matches("blocked: "))}));
                }
            }
        }
        Ok(
            json!({"enabled":config.enabled,"concurrencyCap":config.concurrency_cap,"workers":workers,"needsYou":needs_you}),
        )
    }
}

/// Production runtime adapter. Session files are created/attached by exact reserved id.
pub struct HostWorkers {
    hub: Arc<crate::sessions::SessionHub>,
    layout: Layout,
    runtimes: Mutex<HashMap<String, Arc<AgentSessionRuntime>>>,
}
impl HostWorkers {
    pub fn new(hub: Arc<crate::sessions::SessionHub>, layout: Layout) -> Self {
        Self {
            hub,
            layout,
            runtimes: Mutex::new(HashMap::new()),
        }
    }
}
impl Workers for HostWorkers {
    fn live<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.runtimes.lock().await.contains_key(id) })
    }
    fn launch<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let mut runtimes = self.runtimes.lock().await;
            if runtimes.contains_key(id) {
                return Ok(());
            }
            let services = self
                .hub
                .core_services_for(&project.root)
                .await
                .map_err(|e| e.to_string())?;
            let existing = oino_session::SessionManager::find_by_id(
                &project.root,
                id,
                self.hub.sessions_root(),
            )
            .map_err(|e| e.to_string())?;
            let attaching = existing.is_some();
            let source =
                existing.map_or_else(|| SessionSource::NewWithId(id.into()), SessionSource::Open);
            let runtime = Arc::new(
                AgentSessionRuntime::create_for_hub(
                    services,
                    SessionOptions {
                        source,
                        custom_tools: vec![crate::superagent_session::worker_board(
                            project.clone(),
                            id.into(),
                        )],
                        session_dir: Some(self.hub.sessions_root().to_path_buf()),
                        name: Some(format!("Worker {}", task.id)),
                        ..SessionOptions::default()
                    },
                )
                .await
                .map_err(|e| e.to_string())?,
            );
            let session = runtime.session();
            session.set_unattended();
            session.set_superagent_worker();
            let layout = self.layout.clone();
            let project_for_approval = project.clone();
            let task_id = task.id.clone();
            let worker_id = id.to_owned();
            session.set_worker_approval_transition(Arc::new(move |request, park| {
                let command = request
                    .arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let question = format!(
                    "Approval needed: {} (risk: {:?}, ceiling: {:?}; {}). Command: {}",
                    request.tool, request.risk_level, request.ceiling, request.reason, command
                );
                dispatcher::approval_transition(
                    &layout,
                    project_for_approval.clone(),
                    &task_id,
                    &worker_id,
                    park.then_some(question.as_str()),
                )
                .map_err(|e| e.to_string())
            }));
            let ceiling = serde_json::from_value(json!(dispatcher::autonomy(task)))
                .map_err(|e| e.to_string())?;
            session.set_autonomy(ceiling);
            session.with_manager(|m| m.append_custom_entry("oino:superagent-worker", Some(json!({"projectId":project.slug,"taskId":task.id,"author":"superagent"})))).map_err(|e| e.to_string())?;
            session
                .with_manager(|m| m.materialize())
                .map_err(|e| e.to_string())?;
            self.hub.announce_worker(id).await;
            let prompt = format!(
                "Work ticket {} in project {}. Your claim is already reserved for session {}. Use the kanboard tool to read and transition the ticket. The tool carries your exact session id; do not override it with shell environment flags. Work unattended at the ticket autonomy. Finish to in_review with a summary, or move to blocked with a question. Never leave the ticket in_progress.\n\n{}\n{}",
                task.id, project.slug, id, task.title, task.body
            );
            if !attaching {
                session
                    .prompt(
                        prompt,
                        PromptOptions {
                            source: InputSource::Extension,
                            ..PromptOptions::default()
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
            }
            crate::superagent_session::register(Arc::clone(&runtime));
            runtimes.insert(id.into(), runtime);
            Ok(())
        })
    }
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            runtime.session().abort().await;
            self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            runtime.dispose().await;
            Ok(())
        })
    }
    fn relay<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self
                .runtimes
                .lock()
                .await
                .get(id)
                .cloned()
                .ok_or("Worker is not live")?;
            let session = runtime.session();
            session
                .with_manager(|m| {
                    m.append_custom_entry(
                        "oino:relay",
                        Some(json!({"author":"superagent","text":text})),
                    )
                })
                .map_err(|e| e.to_string())?;
            session.send_custom_message(oino_core::extensions::host::CustomMessageDraft {
            custom_type: "oino:superagent-relay".into(), content: oino_ai::UserContentField::Text(text.into()),
            display: true, details: Some(json!({"author":"superagent", "oinoProvenance":{"kind":"superagent","author":"superagent"}})),
        }, true, Some(oino_core::extensions::host::Delivery::FollowUp)).await.map_err(|e| e.to_string())?;
            Ok(())
        })
    }
    fn retire<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            if let Some(runtime) = runtime {
                runtime.dispose().await;
            }
        })
    }
    fn approvals<'a>(
        &'a self,
        id: &'a str,
    ) -> BoxFuture<'a, Vec<oino_core::session::permission::ApprovalRequest>> {
        Box::pin(async move {
            self.runtimes
                .lock()
                .await
                .get(id)
                .map_or_else(Vec::new, |r| r.session().pending_approvals())
        })
    }
    fn approval_response<'a>(
        &'a self,
        id: &'a str,
        response: oino_core::session::permission::ApprovalResponse,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            self.runtimes
                .lock()
                .await
                .get(id)
                .ok_or("Worker is not live")?
                .session()
                .approval_response(response)
        })
    }
    fn progress<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Value> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned();
            match runtime {
                Some(runtime) => {
                    let session = runtime.session();
                    let tool = session
                        .last_assistant_message()
                        .and_then(|m| serde_json::to_value(m).ok())
                        .and_then(|m| {
                            m["content"].as_array().and_then(|content| {
                                content
                                    .iter()
                                    .rev()
                                    .find(|b| b["type"] == "toolCall")
                                    .and_then(|b| b["name"].as_str())
                                    .map(str::to_owned)
                            })
                        });
                    json!({"step":format!("{:?}",session.run_state()).to_lowercase(),"tool":tool,"cost":session.session_stats().cost})
                }
                None => json!({"step":null,"tool":null,"cost":null}),
            }
        })
    }
    fn ended<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned();
            let Some(runtime) = runtime else {
                return Some(INTERRUPTED.into());
            };
            let session = runtime.session();
            if session.is_streaming() || session.pending_message_count() > 0 {
                None
            } else {
                Some(session.last_assistant_text().unwrap_or_default())
            }
        })
    }
}

static DISPATCHER: OnceLock<Arc<Dispatcher>> = OnceLock::new();
pub fn current() -> Result<&'static Arc<Dispatcher>> {
    DISPATCHER.get().ok_or_else(|| {
        fail(
            "E_DISPATCHER_UNAVAILABLE",
            "This host does not own dispatch; connect to the dispatcher host",
        )
    })
}
/// Install before serving clients. A second host can serve other operations but not dispatch.
pub async fn install(hub: Arc<crate::sessions::SessionHub>, config: &Path) -> Result<()> {
    let layout = Layout::from_env().map_err(storage)?;
    let dispatcher = Dispatcher::acquire(
        layout.clone(),
        config.to_path_buf(),
        Arc::new(HostWorkers::new(hub, layout)),
    )
    .await?;
    DISPATCHER
        .set(Arc::clone(&dispatcher))
        .map_err(|_| fail("E_DISPATCHER_OWNER", "Dispatcher already installed"))?;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            if let Err(e) = dispatcher.tick().await {
                tracing::warn!(%e, "superagent tick failed");
            }
        }
    });
    Ok(())
}

/// Called only after the host's normal client authentication.
pub async fn request(req: unipi_proto::Request) -> Result<Value> {
    use unipi_proto::Request;
    let dispatcher = current()?;
    match req {
        Request::SuperagentStatus => dispatcher.status().await,
        Request::SuperagentConfigure {
            enabled,
            concurrency_cap,
        } => serde_json::to_value(
            dispatcher
                .configure(Config {
                    enabled,
                    concurrency_cap,
                })
                .await?,
        )
        .map_err(storage),
        Request::SuperagentStart {
            project_id,
            task_id,
        } => Ok(json!({"sessionId":dispatcher.start(&project_id, &task_id).await?})),
        Request::SuperagentStop {
            project_id,
            task_id,
        } => Ok(json!({"stopped":dispatcher.stop(&project_id, &task_id).await?})),
        Request::SuperagentRelay {
            project_id,
            task_id,
            text,
        } => dispatcher.relay(&project_id, &task_id, &text).await,
        Request::SuperagentApprovalResponse {
            project_id,
            task_id,
            worker_session_id,
            request_id,
            decision,
        } => {
            let decision = serde_json::from_value(json!(decision)).map_err(storage)?;
            dispatcher
                .approval_response(
                    &project_id,
                    &task_id,
                    &worker_session_id,
                    oino_core::session::permission::ApprovalResponse {
                        request_id,
                        decision,
                    },
                )
                .await?;
            Ok(json!({"accepted":true}))
        }
        _ => Err(fail("E_SCHEMA", "Unknown superagent operation")),
    }
}
