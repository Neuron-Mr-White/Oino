//! Machine-wide Superagent dispatcher. The kernel lock covers the entire owner lifetime;
//! the async transaction lock covers watcher/manual reservation through session launch.
use crate::superagent_presets::{Overrides, Preset, Store};
use futures_util::future::BoxFuture;
use oino_core::{
    runtime::{AgentSessionRuntime, SessionOptions, SessionSource},
    session::{InputSource, PromptOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;
use unipi_kanboard::{
    board::Board,
    dispatcher,
    model::{RunOwner, Status, Task},
    Layout, Project,
};

pub const INTERRUPTED: &str = "Worker was interrupted; move to todo to retry";

/// Disposes a retired/stopped worker runtime, announcing it on the session stream first.
/// A bare `dispose` never ends the streams of backends over this runtime (an open chat's
/// host feed), so they would keep following a dead runtime: no live events, sends rejected
/// as "already processing". `SessionEvicted` ends those taps; the app reopens the chat on
/// a fresh runtime.
async fn dispose_worker(runtime: &AgentSessionRuntime) {
    let session = runtime.session();
    session
        .hub()
        .publish(oino_core::events::OinoEvent::SessionEvicted {
            session_id: session.session_id(),
        });
    runtime.dispose().await;
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery { Queue, Steer }
impl Delivery {
    pub fn parse(value: &str) -> Result<Self> {
        match value { "queue" => Ok(Self::Queue), "steer" => Ok(Self::Steer), _ => Err(fail("E_SCHEMA", "delivery must be queue or steer")) }
    }
}

fn lead_brief(project: &Project, tasks: &[Task]) -> String {
    let tickets = tasks.iter().map(|task| format!("{} — {}\n{}", task.id, task.title, task.body.chars().take(2000).collect::<String>())).collect::<Vec<_>>().join("\n\n");
    format!("Lead work for project {}. Claim each ticket with your kanboard tool, work them using your mode to parallelise where appropriate, and finish each to in_review with a summary or to blocked with a question. Never leave a ticket in_progress.\n\n{}", project.slug, tickets)
}

#[cfg(test)]
mod lead_tests {
    #[test]
    fn brief_contains_all_ticket_details_and_instructions() {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let layout = unipi_kanboard::Layout::with_home(home.path().to_owned());
        let project = unipi_kanboard::Project::create(&layout, root.path(), Some("test"), Some("T")).unwrap();
        let common = unipi_kanboard::commands::Common::new(unipi_kanboard::model::Actor::User, unipi_kanboard::model::ChainGate::InReview);
        let mut tasks = Vec::new();
        for (title, body) in [("Alpha", "Body A"), ("Beta", "Body B")] {
            let value = unipi_kanboard::commands::add(&layout, project.clone(), &common, title, Some(body), Some(unipi_kanboard::model::Status::Todo), unipi_kanboard::model::Priority::Medium, &[], &[], &[], false).unwrap();
            tasks.push(unipi_kanboard::board::Board::open(&layout, project.clone()).unwrap().get(value["id"].as_str().unwrap()).unwrap());
        }
        let brief = super::lead_brief(&project, &tasks);
        for text in [&tasks[0].id, &tasks[1].id, "Alpha", "Beta", "Body A", "Body B", "Claim each ticket", "parallelise", "in_review", "blocked"] { assert!(brief.contains(text), "missing {text}"); }
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
    fn launch_preset<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
        _preset: &'a Preset,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.launch(project, task, id)
    }
    fn launch_lead<'a>(&'a self, project: &'a Project, tasks: &'a [Task], id: &'a str, preset: &'a Preset) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.launch_preset(project, &tasks[0], id, preset)
    }
    fn relay_delivery<'a>(&'a self, id: &'a str, text: &'a str, _delivery: Delivery) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.relay(id, text)
    }
    fn set_mode<'a>(&'a self, _id: &'a str, _mode: &'a str) -> BoxFuture<'a, std::result::Result<(), String>> { Box::pin(async { Ok(()) }) }
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
    preferences: Mutex<Store>,
    preferences_path: PathBuf,
    launch_presets: std::sync::Mutex<HashMap<String, Preset>>,
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
        let preferences_path = layout.home.join("superagent-preferences.json");
        let preferences = Store::load(&preferences_path).map_err(storage)?;
        let this = Arc::new(Self {
            preferences: Mutex::new(preferences),
            preferences_path,
            launch_presets: std::sync::Mutex::new(HashMap::new()),
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
            if project.archived {
                continue;
            }
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
    pub async fn configure_project(&self, project_id: &str, auto_start: bool) -> Result<Value> {
        Project::load(&self.layout, project_id).map_err(storage)?;
        let mut prefs = self.preferences.lock().await;
        let mut next = prefs.clone();
        next.projects.insert(project_id.into(), auto_start);
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"projectId":project_id,"autoStart":auto_start}))
    }
    pub async fn presets_list(&self) -> Value {
        let prefs = self.preferences.lock().await;
        json!({"defaultPresetId":prefs.default_preset_id,"presets":prefs.presets})
    }
    pub async fn preset_save(&self, preset: Preset) -> Result<Value> {
        preset.validate().map_err(|e| fail("E_SCHEMA", e))?;
        let mut prefs = self.preferences.lock().await;
        let mut next = prefs.clone();
        next.presets.retain(|p| p.id != preset.id);
        next.presets.push(preset.clone());
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"preset":preset}))
    }
    pub async fn preset_delete(&self, id: &str) -> Result<Value> {
        let mut prefs = self.preferences.lock().await;
        if prefs.default_preset_id == id {
            return Err(fail("E_SCHEMA", "Cannot delete the default preset"));
        }
        let mut next = prefs.clone();
        let before = next.presets.len();
        next.presets.retain(|p| p.id != id);
        let deleted = before != next.presets.len();
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"deleted":deleted}))
    }
    pub async fn set_mode(&self, project_id: &str, mode: &str) -> Result<Value> {
        if !matches!(mode, "none" | "goal" | "ralph" | "swarm" | "graph") { return Err(fail("E_SCHEMA", "Invalid long-horizon mode")); }
        let _transaction = self.transaction.lock().await;
        let id = self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(_, p)| p.as_str() == project_id).map(|(id, _)| id.clone()).ok_or_else(|| fail("E_WORKER_CLAIM", "Project has no active lead"))?;
        self.workers.set_mode(&id, mode).await.map_err(|e| fail("E_MODE", e))?;
        if let Some(preset) = self.launch_presets.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_mut(&id) { preset.mode = mode.into(); }
        Ok(json!({"sessionId":id,"mode":mode}))
    }
    pub async fn preset_set_default(&self, id: &str) -> Result<Value> {
        let mut prefs = self.preferences.lock().await;
        if !prefs.presets.iter().any(|p| p.id == id) {
            return Err(fail("E_SCHEMA", "Unknown preset"));
        }
        let mut next = prefs.clone();
        next.default_preset_id = id.into();
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"defaultPresetId":id}))
    }
    /// Shared watcher/manual transaction. Reservation is durable before runtime creation.
    pub async fn start(&self, project_id: &str, task_id: &str) -> Result<String> {
        self.start_with_preset(project_id, task_id, None, Overrides::default())
            .await
    }
    pub async fn start_with_preset(
        &self,
        project_id: &str,
        task_id: &str,
        preset_id: Option<&str>,
        overrides: Overrides,
    ) -> Result<String> {
        self.start_tasks(project_id, &[task_id.to_owned()], preset_id, overrides).await
    }
    pub async fn start_tasks(&self, project_id: &str, task_ids: &[String], preset_id: Option<&str>, overrides: Overrides) -> Result<String> {
        self.start_transaction(project_id, task_ids, false, preset_id, overrides).await
    }
    async fn start_transaction(
        &self,
        project_id: &str,
        task_ids: &[String],
        watcher: bool,
        preset_id: Option<&str>,
        overrides: Overrides,
    ) -> Result<String> {
        let config = self.transaction.lock().await;
        if task_ids.is_empty() || task_ids.iter().collect::<std::collections::HashSet<_>>().len() != task_ids.len() { return Err(fail("E_SCHEMA", "taskIds must be nonempty and unique")); }
        if watcher && !config.enabled {
            return Err(fail(
                "E_WATCHER_OFF",
                "Watcher is disabled; manually start a ticket or enable it",
            ));
        }
        let preferences = self.preferences.lock().await;
        if watcher
            && !preferences
                .projects
                .get(project_id)
                .copied()
                .unwrap_or(false)
        {
            return Err(fail("E_WATCHER_OFF", "Project autoStart is disabled"));
        }
        let preset = preferences
            .resolve(preset_id, overrides)
            .map_err(|e| fail("E_SCHEMA", e))?;
        drop(preferences);
        let claims = self.claims()?;
        // Live leads hold their project even after their tickets finished (one lead per project);
        // a lead of an archived project no longer counts (OIN-21).
        let archived: std::collections::HashSet<String> = self
            .layout
            .list_projects()
            .map_err(storage)?
            .into_iter()
            .filter(|p| p.archived)
            .map(|p| p.slug)
            .collect();
        let mut occupied: HashMap<String, String> = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, slug)| !archived.contains(slug.as_str()))
            .map(|(id, slug)| (id.clone(), slug.clone()))
            .collect();
        for (project, task) in claims {
            if let Some(run) = task.run {
                occupied.insert(run.session, project.slug);
            }
        }
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        if project.archived {
            return Err(fail("E_WORKER_REFUSED", "Project is archived"));
        }
        if let Some((id, _)) = occupied.iter().find(|(_, p)| p.as_str() == project_id) {
            if !self.workers.live(id).await || self.workers.ended(id).await.is_some() { return Err(fail("E_WORKER_NOT_LIVE", "Lead has ended; retry after it is retired")); }
            let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
            let ids: Vec<String> = task_ids.iter().filter(|task_id| !board.get(task_id).ok().is_some_and(|task| dispatcher::matches_claim(&task, id))).cloned().collect();
            if ids.is_empty() { return Ok(id.clone()); }
            let tasks = dispatcher::reserve_many(&self.layout, project.clone(), &ids, id).map_err(|e| fail("E_WORKER_REFUSED", e))?;
            let text = lead_brief(&project, &tasks);
            for task in &tasks {
                dispatcher::relay(&self.layout, project.clone(), &task.id, id, &text, &uuid::Uuid::now_v7().to_string()).map_err(storage)?;
            }
            if let Err(e) = self.workers.relay_delivery(id, &text, Delivery::Queue).await {
                for task in &tasks { dispatcher::block(&self.layout, project.clone(), &task.id, id, &format!("Lead delivery failed: {e}; move to todo to retry")).map_err(storage)?; }
                return Err(fail("E_RELAY_DELIVERY", e));
            }
            return Ok(id.clone());
        }
        if occupied.len() >= config.concurrency_cap {
            return Err(fail(
                "E_WORKER_CAP",
                "Worker cap reached; wait for a worker or raise concurrencyCap",
            ));
        }
        let id = uuid::Uuid::now_v7().to_string();
        let tasks = dispatcher::reserve_many(&self.layout, project.clone(), task_ids, &id)
            .map_err(|e| fail("E_WORKER_REFUSED", e))?;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), project_id.into());
        if let Err(e) = self
            .workers
            .launch_lead(&project, &tasks, &id, &preset)
            .await
        {
            for task in &tasks {
                dispatcher::block(&self.layout, project.clone(), &task.id, &id, &format!("Worker could not start: {e}; move to todo to retry")).map_err(storage)?;
            }
            self.active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            return Err(fail("E_WORKER_LAUNCH", e));
        }
        self.launch_presets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), preset);
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
                    let question = if task.status == Status::Blocked {
                        task.activity.iter().rev().find_map(|entry| entry.text.strip_prefix("blocked: ")).unwrap_or("Worker ended while blocked; move to todo to retry")
                    } else if last.trim().is_empty() {
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
                    self.launch_presets.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
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
            let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
            let tasks = board.tasks().map_err(storage)?;
            let ids: Vec<String> = tasks.iter().filter(|task| unipi_kanboard::deps::is_ready(task, &board.dep_lookup(&tasks), unipi_kanboard::model::ChainGate::InReview)).map(|task| task.id.clone()).collect();
            if !ids.is_empty() {
                let _ = self.start_transaction(&project.slug, &ids, true, None, Overrides::default()).await;
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
        for (claimed_project, claimed) in self.claims()? {
            if claimed_project.slug == project_id && claimed.run.as_ref().is_some_and(|r| r.session == run.session) {
                dispatcher::block(&self.layout, claimed_project, &claimed.id, &run.session, "Worker stopped; move to todo to retry").map_err(storage)?;
            }
        }
        Ok(true)
    }
    pub async fn relay(&self, project_id: &str, task_id: &str, text: &str) -> Result<Value> {
        self.relay_to_lead(project_id, text, &[task_id.to_owned()], Delivery::Queue, false).await
    }
    pub async fn relay_to_lead(&self, project_id: &str, text: &str, task_ids: &[String], delivery: Delivery, priority: bool) -> Result<Value> {
        let _transaction = self.transaction.lock().await;
        if text.trim().is_empty() { return Err(fail("E_SCHEMA", "Relay text must not be empty")); }
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        let id = self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(_, p)| p.as_str() == project_id).map(|(id, _)| id.clone()).ok_or_else(|| fail("E_WORKER_CLAIM", "Project has no active lead"))?;
        if !self.workers.live(&id).await || self.workers.ended(&id).await.is_some() { return Err(fail("E_WORKER_NOT_LIVE", "Lead has ended; refresh the board before relaying")); }
        let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
        let ids: Vec<String> = if task_ids.is_empty() { board.tasks().map_err(storage)?.into_iter().filter(|t| dispatcher::matches_claim(t, &id)).map(|t| t.id).collect() } else { task_ids.to_vec() };
        let text = if priority { format!("URGENT: schedule this first / add it as the first graph item\n\n{text}") } else { text.to_owned() };
        let comment = uuid::Uuid::now_v7().to_string();
        for task_id in &ids {
            dispatcher::relay(&self.layout, project.clone(), task_id, &id, &text, &comment).map_err(storage)?;
        }
        let mut reply = json!({"commentId": if ids.is_empty() { None } else { Some(comment) }, "sessionId":id});
        match self.workers.relay_delivery(&id, &text, delivery).await {
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
        let mut leads = Vec::new();
        let mut projects = Vec::new();
        let mut review = Vec::new();
        let prefs = self.preferences.lock().await;
        let mut needs_you = Vec::new();
        for project in self.layout.list_projects().map_err(storage)? {
            if project.archived {
                continue;
            }
            let mut lead = None;
            let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
            let tasks = board.tasks().map_err(storage)?;
            let ready = tasks.iter().filter(|task| unipi_kanboard::deps::is_ready(task, &board.dep_lookup(&tasks), unipi_kanboard::model::ChainGate::InReview)).count();
            for task in tasks {
                if task.status == Status::Archived {
                    continue;
                }
                if let Some(run) = &task.run {
                    if run.owner == RunOwner::Dispatcher {
                        let progress = self.workers.progress(&run.session).await;
                        let preset = self
                            .launch_presets
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get(&run.session)
                            .cloned();
                        if self.workers.live(&run.session).await && self.workers.ended(&run.session).await.is_none() {
                        lead = Some(run.session.clone());
                        if let Some(existing) = leads.iter_mut().find(|entry: &&mut Value| entry["sessionId"] == run.session) {
                            if let Some(ids) = existing["ticketIds"].as_array_mut() { ids.push(json!(task.id)); }
                            existing["progress"]["running"] = json!(existing["ticketIds"].as_array().map_or(0, Vec::len));
                        } else {
                            leads.push(json!({"projectId":project.slug,"sessionId":run.session,"presetId":preset.as_ref().map(|p| &p.id),"model":progress.get("model").filter(|v| !v.is_null()).cloned().or_else(|| preset.as_ref().and_then(|p| p.model.as_ref()).map(|v| json!(v))),"thinking":progress.get("thinking").filter(|v| !v.is_null()).cloned().or_else(|| preset.as_ref().and_then(|p| p.thinking.as_ref()).map(|v| json!(v))),"autonomy":progress.get("autonomy").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(preset.as_ref().map_or("high", |p| &p.autonomy))),"mode":preset.as_ref().map_or("none", |p| &p.mode),"ticketIds":[task.id],"step":progress["step"],"tool":progress["tool"],"startedAt":run.started,"cost":progress["cost"],"progress":{"running":1,"queued":0,"done":0}}));
                        }
                        }
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
                if task.status == Status::InReview {
                    review.push(
                        json!({"projectId":project.slug,"taskId":task.id,"status":"in_review"}),
                    );
                } else if task.status == Status::Blocked && task.run.is_none() {
                    if let Some(question) = task
                        .activity
                        .iter()
                        .rev()
                        .find(|a| a.text.starts_with("blocked:"))
                        .map(|a| a.text.trim_start_matches("blocked: "))
                        .filter(|q| !q.trim().is_empty())
                    {
                        needs_you.push(json!({"projectId":project.slug,"taskId":task.id,"status":"blocked","question":question}));
                    }
                }
            }
            projects.push(json!({"projectId":project.slug,"name":project.name,"autoStart":prefs.projects.get(&project.slug).copied().unwrap_or(false),"lead":lead,"ready":ready}));
        }
        Ok(
            json!({"enabled":config.enabled,"concurrencyCap":config.concurrency_cap,"workers":workers,"leads":leads,"needsYou":needs_you,"review":review,"projects":projects}),
        )
    }
}

async fn apply_mode(session: &oino_core::session::AgentSession, mode: &str) -> std::result::Result<(), String> {
    use oino_core::extensions::{api::CallKind, host::HostOp};
    if mode == "none" && session.runner().command("lh-mode").is_none() { return Ok(()); }
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    let result = session.host_call("unipi.long-horizon", &cx, HostOp::InvokeCommand { name: "lh-mode".into(), args: json!({"mode":mode}).to_string() }).await.map_err(|e| e.to_string())?;
    if result["ok"] != true { return Err(result["reason"].as_str().unwrap_or("Mode switch refused").into()); }
    Ok(())
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
            let preset = Store::default().resolve(None, Overrides::default())?;
            self.launch_preset(project, task, id, &preset).await
        })
    }
    fn launch_preset<'a>(
        &'a self,
        project: &'a Project,
        task: &'a Task,
        id: &'a str,
        preset: &'a Preset,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.launch_lead(project, std::slice::from_ref(task), id, preset)
    }
    fn launch_lead<'a>(&'a self, project: &'a Project, tasks: &'a [Task], id: &'a str, preset: &'a Preset) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let task = &tasks[0];
            let mut runtimes = self.runtimes.lock().await;
            if runtimes.contains_key(id) {
                return Ok(());
            }
            let services = self
                .hub
                .core_services_for(&project.root)
                .await
                .map_err(|e| e.to_string())?;
            let model = preset
                .model
                .as_ref()
                .map(|name| {
                    let (provider, id) = name.split_once('/').ok_or("model must be provider/id")?;
                    services
                        .models
                        .find(provider, id)
                        .ok_or_else(|| format!("Unknown model: {name}"))
                })
                .transpose()?;
            let thinking_level = preset
                .thinking
                .as_ref()
                .map(|level| {
                    level
                        .parse()
                        .map_err(|_| format!("Invalid thinking level: {level}"))
                })
                .transpose()?;
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
                        model,
                        thinking_level,
                        custom_tools: vec![crate::superagent_session::worker_board(
                            project.clone(),
                            id.into(),
                        )],
                        session_dir: Some(self.hub.sessions_root().to_path_buf()),
                        name: Some(format!("Lead · {}", if project.name.is_empty() { &project.slug } else { &project.name })),
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
                let board = Board::open(&layout, project_for_approval.clone()).map_err(|e| e.to_string())?;
                for task in board.tasks().map_err(|e| e.to_string())? {
                    if dispatcher::matches_claim(&task, &worker_id) {
                        dispatcher::approval_transition(&layout, project_for_approval.clone(), &task.id, &worker_id, park.then_some(question.as_str())).map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            }));
            let ceiling =
                serde_json::from_value(json!(preset.autonomy)).map_err(|e| e.to_string())?;
            session.set_autonomy(ceiling);
            session.with_manager(|m| m.append_custom_entry("oino:superagent-worker", Some(json!({"projectId":project.slug,"projectName":project.name,"taskId":task.id,"author":"superagent"})))).map_err(|e| e.to_string())?;
            session
                .with_manager(|m| m.materialize())
                .map_err(|e| e.to_string())?;
            self.hub.announce_worker(id).await;
            apply_mode(&session, &preset.mode).await?;
            let prompt = lead_brief(project, tasks);
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
    fn set_mode<'a>(&'a self, id: &'a str, mode: &'a str) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.get(id).cloned().ok_or("Lead is not live")?;
            apply_mode(&runtime.session(), mode).await
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
            dispose_worker(&runtime).await;
            Ok(())
        })
    }
    fn relay<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.relay_delivery(id, text, Delivery::Queue)
    }
    fn relay_delivery<'a>(&'a self, id: &'a str, text: &'a str, delivery: Delivery) -> BoxFuture<'a, std::result::Result<(), String>> {
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
        }, true, Some(match delivery { Delivery::Queue => oino_core::extensions::host::Delivery::FollowUp, Delivery::Steer => oino_core::extensions::host::Delivery::Steer })).await.map_err(|e| e.to_string())?;
            Ok(())
        })
    }
    fn retire<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let runtime = self.runtimes.lock().await.remove(id);
            crate::superagent_session::unregister(id);
            if let Some(runtime) = runtime {
                dispose_worker(&runtime).await;
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
                    json!({"step":format!("{:?}",session.run_state()).to_lowercase(),"tool":tool,"cost":session.session_stats().cost,"model":session.model().map(|m| format!("{}/{}", m.provider, m.id)),"thinking":session.thinking_level().to_string(),"autonomy":session.autonomy()})
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
        Request::SuperagentConfigureProject {
            project_id,
            auto_start,
        } => dispatcher.configure_project(&project_id, auto_start).await,
        Request::SuperagentSetMode { project_id, mode } => dispatcher.set_mode(&project_id, &mode).await,
        Request::SuperagentPresetsList => Ok(dispatcher.presets_list().await),
        Request::SuperagentPresetsSave { preset } => {
            dispatcher
                .preset_save(serde_json::from_value(preset).map_err(|e| fail("E_SCHEMA", e))?)
                .await
        }
        Request::SuperagentPresetsDelete { preset_id } => {
            dispatcher.preset_delete(&preset_id).await
        }
        Request::SuperagentPresetsSetDefault { preset_id } => {
            dispatcher.preset_set_default(&preset_id).await
        }
        Request::SuperagentStart {
            project_id,
            task_id,
            task_ids,
            preset_id,
            overrides,
        } => {
            let tasks = if let Some(task) = task_id {
                if !task_ids.is_empty() {
                    return Err(fail("E_SCHEMA", "Use taskId or taskIds, not both"));
                }
                vec![task]
            } else {
                task_ids
            };
            if tasks.is_empty() { return Err(fail("E_SCHEMA", "taskIds must not be empty")); }
            let overrides = overrides
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| fail("E_SCHEMA", e))?
                .unwrap_or_default();
            Ok(
                json!({"sessionId":dispatcher.start_tasks(&project_id, &tasks, preset_id.as_deref(), overrides).await?}),
            )
        }
        Request::SuperagentStop {
            project_id,
            task_id,
        } => Ok(json!({"stopped":dispatcher.stop(&project_id, &task_id).await?})),
        Request::SuperagentRelay { project_id, task_id, text, mut task_ids, delivery, priority } => {
            if let Some(task) = task_id { if !task_ids.is_empty() { return Err(fail("E_SCHEMA", "Use taskId or taskIds, not both")); } task_ids.push(task); }
            dispatcher.relay_to_lead(&project_id, &text, &task_ids, Delivery::parse(delivery.as_deref().unwrap_or("queue"))?, priority).await
        },
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
