//! Machine-wide Superagent dispatcher. The kernel lock covers the entire owner lifetime;
//! the async transaction lock covers watcher/manual reservation through session launch.
use crate::superagent_presets::{Overrides, Preset, ProjectLead, Store};
use futures_util::future::BoxFuture;
use oino_core::runtime::AgentSessionRuntime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
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

pub(crate) use crate::superagent_brief::lead_brief;
pub use crate::superagent_workers::HostWorkers;
#[allow(
    unused_imports,
    reason = "re-export so worker call sites read the same as before the split"
)]
pub(crate) use crate::superagent_workers::{
    apply_mode, has_long_horizon_owner, long_horizon_workspace_dir, paused_graph_for_lead,
};

pub const INTERRUPTED: &str = "Worker was interrupted; move to todo to retry";

/// Disposes a retired/stopped worker runtime, announcing it on the session stream first.
/// A bare `dispose` never ends the streams of backends over this runtime (an open chat's
/// host feed), so they would keep following a dead runtime: no live events, sends rejected
/// as "already processing". `SessionEvicted` ends those taps; the app reopens the chat on
/// a fresh runtime.
pub(crate) async fn dispose_worker(runtime: &AgentSessionRuntime) {
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
#[allow(
    clippy::needless_pass_by_value,
    reason = "Owned error/value adapter matches its consuming callback and avoids borrow-only wrappers at every call site."
)]
pub(crate) fn fail(code: &'static str, message: impl ToString) -> DispatchError {
    DispatchError {
        code,
        message: message.to_string(),
    }
}
pub(crate) fn storage(e: impl ToString) -> DispatchError {
    fail("E_SUPERAGENT_STORAGE", e)
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockerAction {
    pub kind: &'static str,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Blocker {
    pub code: &'static str,
    pub message: String,
    pub action: BlockerAction,
}

fn project_blockers(
    project_id: &str,
    project: Option<&Project>,
    enabled: bool,
    auto_start: bool,
) -> Vec<Blocker> {
    let blocker = |code, message: String, kind, label: String| Blocker {
        code,
        message,
        action: BlockerAction { kind, label },
    };
    let Some(project) = project else {
        return vec![blocker(
            "not_registered",
            "This project has no board yet. Set one up to add tasks and start work.".to_owned(),
            "register",
            "Set up board".to_owned(),
        )];
    };
    if project.archived {
        return vec![blocker(
            "archived",
            format!(
                "Project {project_id} is archived. Unarchive it on the Kanboard dashboard and retry."
            ),
            "unarchive",
            "Unarchive project".into(),
        )];
    }
    let mut blockers = Vec::new();
    if !auto_start {
        blockers.push(blocker(
            "project_auto_off",
            format!(
                "Project {project_id} Auto is off, so ready tickets will not start automatically."
            ),
            "enable_project_auto",
            "Enable project Auto".into(),
        ));
    }
    if !enabled {
        blockers.push(blocker(
            "global_auto_off",
            "Global Auto-start is off, so ready tickets will not start automatically.".into(),
            "enable_auto_start",
            "Enable Auto-start".into(),
        ));
    }
    blockers
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PausedGraphPolicy {
    #[default]
    Report,
    Reuse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedGraph {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    #[serde(default, skip_serializing)]
    pub concurrency_cap: usize,
    #[serde(default)]
    pub paused_graph_policy: PausedGraphPolicy,
    /// `superagent.lead.*` (OIN-80): retry budget, fallback model, warm unload,
    /// question timeout. Defaults keep pre-OIN-80 configs loading.
    #[serde(default)]
    pub lead: crate::superagent_leads::LeadSettings,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            concurrency_cap: 1,
            paused_graph_policy: PausedGraphPolicy::Report,
            lead: crate::superagent_leads::LeadSettings::default(),
        }
    }
}
impl Config {
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Queue,
    Steer,
}
impl Delivery {
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "queue" => Ok(Self::Queue),
            "steer" => Ok(Self::Steer),
            _ => Err(fail("E_SCHEMA", "delivery must be queue or steer")),
        }
    }
}

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
    fn launch_lead<'a>(
        &'a self,
        project: &'a Project,
        tasks: &'a [Task],
        id: &'a str,
        preset: &'a Preset,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        match tasks.first() {
            Some(task) => self.launch_preset(project, task, id, preset),
            // A wake/reattach with no tickets (OIN-81 branch (2), lead wake):
            // no launch to run, the session already exists by id.
            None => Box::pin(async move { Ok(()) }),
        }
    }
    fn relay_delivery<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
        _delivery: Delivery,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        self.relay(id, text)
    }
    fn set_mode<'a>(
        &'a self,
        _id: &'a str,
        _mode: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
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
    fn agent_dir(&self) -> Option<&Path> {
        None
    }
    // --- OIN-80 lead-machine additions (default impls keep `Fake` and test doubles compiling) ---
    /// The lead's turn outcome: `Some("provider error: …")` after an errored turn,
    /// `Some("")`/`Some(text)` on a clean end, `None` while busy.
    fn outcome<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move { self.ended(id).await })
    }
    /// Switch the live lead's model without persisting the user default.
    fn set_model<'a>(
        &'a self,
        _id: &'a str,
        _model: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    /// Continue a lead's turn with a host message (`oino:lead-continue`, invisible).
    fn continue_turn<'a>(
        &'a self,
        id: &'a str,
        text: &'a str,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move { self.relay_delivery(id, text, Delivery::Queue).await })
    }
    /// Dispose the runtime, keep the record (the warm-idle unload).
    fn unload<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// The reserved session's file, captured after a launch for restart resume.
    fn session_file<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Option<std::path::PathBuf>> {
        Box::pin(async { None })
    }
    /// The project's core services (model validation for `switch_model`).
    fn core_services<'a>(
        &'a self,
        _project: &'a Project,
    ) -> BoxFuture<'a, std::result::Result<std::sync::Arc<oino_core::services::Services>, String>>
    {
        Box::pin(async { Err("models are not available in this worker".into()) })
    }
    /// OIN-81: the lead's open `ask_user` dialogs (`Workers::questions`).
    fn questions<'a>(
        &'a self,
        _id: &'a str,
    ) -> BoxFuture<'a, Vec<crate::superagent_questions::LeadQuestion>> {
        Box::pin(async { Vec::new() })
    }
    /// OIN-81: answer an open lead question from the work pane (first-wins
    /// like every dialog). `Err` when the lead is gone or already answered.
    fn answer<'a>(
        &'a self,
        _id: &'a str,
        _request_id: &'a str,
        _answer: oino_core::ui::UiAnswer,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async { Err("Worker is not live".into()) })
    }
    /// Dispatch a lead notification (stall entry or new lead question). Returns
    /// `true` once the notice dispatch ran.
    fn lead_notice<'a>(
        &'a self,
        id: &'a str,
        title: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let Some(hub) = crate::sessions::SessionService::shared().map(|s| s.hub().clone())
            else {
                tracing::debug!("lead_notice: no session service; push skipped");
                return false;
            };
            crate::notify::lead_notice_with_hub(&hub, id, title, message).await
        })
    }
    /// Positive archive evidence for `id` (e.g. session file moved into `<agent_dir>/trash`).
    fn is_session_archived<'a>(
        &'a self,
        id: &'a str,
        session_file: Option<&'a Path>,
    ) -> BoxFuture<'a, bool> {
        let archived = self.agent_dir().is_some_and(|dir| {
            crate::superagent_workers::session_in_trash(&dir.join("trash"), id, session_file)
        });
        Box::pin(async move { archived })
    }
}

pub struct Dispatcher {
    _owner: File,
    pub(crate) layout: Layout,
    config_path: PathBuf,
    pub(crate) transaction: Mutex<Config>,
    pub(crate) workers: Arc<dyn Workers>,
    active: std::sync::Mutex<HashMap<String, String>>,
    preferences: Mutex<Store>,
    preferences_path: PathBuf,
    launch_presets: std::sync::Mutex<HashMap<String, Preset>>,
    /// The durable lead records (`superagent-leads.json`), OIN-80.
    pub(crate) leads: std::sync::Mutex<crate::superagent_leads::LeadStore>,
    pub(crate) leads_path: PathBuf,
    /// OIN-81: lead sessions are exempt from the Settled phone push (F12).
    pub(crate) lead_sessions: std::sync::Mutex<HashSet<String>>,
}
impl Dispatcher {
    /// Reconcile all local durable reservations before accepting any dispatch.
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn acquire(
        layout: Layout,
        config_path: PathBuf,
        workers: Arc<dyn Workers>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&layout.home).map_err(storage)?;
        // The flock can transiently fail when a same-binary test child handed an
        // inherited fd copy to a fork between fork and exec (pre-CLOEXEC, v3_dispatcher
        // flake 2026-11-11): the fd closes when the child's spawn completes, a few
        // hundred ms at most. A real second host holds the lock for its whole lifetime,
        // so a short bounded retry cannot mask it (far under any operator timeout).
        let lock_path = layout.home.join("dispatcher.lock");
        let mut attempt = 0u32;
        let owner = loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(storage)?;
            match file.try_lock() {
                Ok(()) => break file,
                Err(e) if attempt < 10 => {
                    drop(file);
                    attempt += 1;
                    tracing::debug!(attempt, %e, "dispatcher lock held; retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(_) => {
                    return Err(fail(
                        "E_DISPATCHER_OWNER",
                        "Another host owns dispatch; use that host or stop it first",
                    ));
                }
            }
        };
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
        // The legacy per-session continuation map moved into the lead records;
        // `load_or_migrate` folds `dispatcher-continuations.json` in and removes it.
        let leads = crate::superagent_leads::LeadStore::load_or_migrate(&layout)?;
        let leads_path = crate::superagent_leads::LeadStore::path_of(&layout);
        let initial_lead_ids: Vec<String> =
            leads.leads.values().map(|r| r.session_id.clone()).collect();
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
            leads: std::sync::Mutex::new(leads),
            leads_path,
            lead_sessions: std::sync::Mutex::new(HashSet::new()),
        });
        for session_id in &initial_lead_ids {
            this.register_lead_session(session_id);
        }
        for (project, task) in this.claims()? {
            let run = task.run.as_ref().ok_or_else(|| storage("Missing run"))?;
            // OIN-80 restart resume: a claim with a record is not interrupted —
            // the lead re-attaches to the same session id in `resume_interrupted`.
            if this
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .for_project(&project.slug)
                .is_some()
            {
                this.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(run.session.clone(), project.slug);
                continue;
            }
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
        // Q3: resume interrupted Working leads after a restart (same session id).
        this.resume_interrupted().await;
        Ok(this)
    }
    /// Spawn-safe restart resume: re-launch every Working record (attach by id),
    /// send the restart text, mark Waiting/Idle records Unloaded. Runs once from
    /// `acquire`; a launch failure stalls the record without dropping claims.
    async fn resume_interrupted(self: &Arc<Self>) {
        let records: Vec<(String, String)> = {
            let leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads
                .leads
                .values()
                .map(|r| (r.project_id.clone(), r.session_id.clone()))
                .collect()
        };
        for (project_id, session_id) in records {
            let Some(project) = self
                .layout
                .list_projects()
                .unwrap_or_default()
                .into_iter()
                .find(|p| p.slug == project_id)
            else {
                continue;
            };
            let (state, preset) = {
                let leads = self
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(record) = leads.for_project(&project_id) else {
                    continue;
                };
                (record.state, record.preset.clone())
            };
            match state {
                crate::superagent_leads::LeadState::Working => {
                    let tasks: Vec<Task> = self
                        .claims()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(p, t)| {
                            p.slug == project_id
                                && t.run.as_ref().is_some_and(|r| r.session == session_id)
                        })
                        .map(|(_, t)| t)
                        .collect();
                    self.register_lead_session(&session_id);
                    match self
                        .workers
                        .launch_lead(&project, &tasks, &session_id, &preset)
                        .await
                    {
                        Ok(()) => {
                            let session_file = self.workers.session_file(&session_id).await;
                            let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
                            {
                                let mut leads = self
                                    .leads
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                if let Some(record) = leads.leads.get_mut(&project_id) {
                                    record.session_file = session_file;
                                    record.resume_sent_at = Some(crate::superagent_leads::now_ms());
                                    leads.save(&self.leads_path).map_err(storage).ok();
                                }
                            }
                            // Persisted before sending so a crash cannot loop the text.
                            let text = crate::superagent_leads::RESTART_TEXT
                                .replace("{ids}", &ids.join(", "));
                            let _ = self.workers.continue_turn(&session_id, &text).await;
                        }
                        Err(e) => {
                            let now = crate::superagent_leads::now_ms();
                            crate::superagent_leads::enter_stall(
                                self,
                                &project,
                                crate::superagent_leads::HealthKind::Launch,
                                format!("Could not reopen lead: {e}"),
                                now,
                                Some(now + crate::superagent_leads::STALL_REPROBE_MS),
                            )
                            .await
                            .ok();
                        }
                    }
                }
                crate::superagent_leads::LeadState::Waiting
                | crate::superagent_leads::LeadState::Idle => {
                    let mut leads = self
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(record) = leads.leads.get_mut(&project_id) {
                        record.state = crate::superagent_leads::LeadState::Unloaded;
                        leads.save(&self.leads_path).map_err(storage).ok();
                    }
                }
                _ => {}
            }
        }
    }
    pub(crate) fn claims(&self) -> Result<Vec<(Project, Task)>> {
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
    pub async fn paused_graph_policy(&self) -> PausedGraphPolicy {
        self.transaction.lock().await.paused_graph_policy
    }
    /// The lead settings (clamped). Reads the persisted config file, not the
    /// transaction lock: callers hold that lock while dispatching (a `try_lock`
    /// here would silently fall back to the defaults).
    pub fn lead_settings(&self) -> crate::superagent_leads::LeadSettings {
        let config = std::fs::read(&self.config_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value.get("superagent").cloned())
            .and_then(|value| serde_json::from_value::<Config>(value).ok());
        crate::superagent_leads::LeadSettings::sanitized(config.map(|c| c.lead).unwrap_or_default())
    }
    /// The dispatcher's lead store (`superagent-leads.json`); tests read it.
    pub fn leads(&self) -> &std::sync::Mutex<crate::superagent_leads::LeadStore> {
        &self.leads
    }
    /// Shared lead-session registration helper (Item 7): called by fresh launch,
    /// `lead:"reuse"`, wake, and restart paths BEFORE the runtime runs so turn-end
    /// `Settled` push suppression sees the session id from the very first event.
    pub(crate) fn register_lead_session(&self, session_id: &str) {
        self.lead_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id.to_owned());
    }
    pub(crate) fn mark_active(&self, session_id: &str, project_id: &str) {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id.to_owned(), project_id.to_owned());
    }
    /// OIN-81 (F12): is this session a Superagent lead (the Settled push
    /// suppression reads it on every event)?
    pub fn is_lead_session(&self, session_id: &str) -> bool {
        self.lead_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(session_id)
    }
    /// Q11: retire every recorded lead whose session is in `ids` (the hub's
    /// `archive/archive_many` hook; public so tests can drive it directly).
    /// Finding 5: `ids` must be the ids that were ACTUALLY archived
    /// (`SessionsBatchData.done`), not the raw request. Each match revalidates
    /// the record's session id under the transaction lock, so a concurrent
    /// `lead:"new"` (record replaced between the scan and the lock) never
    /// retires the replacement.
    pub async fn sessions_archived(&self, ids: &[String]) {
        let transaction = self.transaction.lock().await;
        let matches: Vec<String> = {
            let leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads
                .leads
                .values()
                .filter(|record| ids.contains(&record.session_id))
                .map(|record| record.project_id.clone())
                .collect()
        };
        for project_id in matches {
            // Revalidate under the lock: the record must still point at an
            // archived session id (a racing lead:"new" changed it).
            let session_id = {
                let leads = self
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                leads.leads.get(&project_id).map(|r| r.session_id.clone())
            };
            let Some(session_id) = session_id else {
                continue;
            };
            if !ids.contains(&session_id) {
                continue;
            }
            let _ = crate::superagent_leads::retire_lead(self, &project_id, "archived").await;
        }
        let mut lead_sessions = self
            .lead_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in ids {
            lead_sessions.remove(id);
        }
        drop(lead_sessions);
        drop(transaction);
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn configure_project(&self, project_id: &str, auto_start: bool) -> Result<Value> {
        Project::load(&self.layout, project_id).map_err(storage)?;
        let mut prefs = self.preferences.lock().await;
        let mut next = prefs.clone();
        next.projects.insert(project_id.into(), auto_start);
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"projectId":project_id,"autoStart":auto_start}))
    }
    /// Perform an action explicitly confirmed by the client.
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn enable(
        &self,
        action: &str,
        project_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<Value> {
        let id = || {
            project_id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| fail("E_SCHEMA", "projectId is required"))
        };
        let result = match action {
            "register" => {
                let path = path
                    .filter(|p| !p.trim().is_empty())
                    .ok_or_else(|| fail("E_SCHEMA", "path is required"))?;
                let path = Path::new(path);
                if !path.is_absolute() || !path.is_dir() {
                    return Err(fail("E_SCHEMA", "path must be an absolute directory"));
                }
                let project =
                    unipi_kanboard::commands::project_add(&self.layout, None, None, None, path)
                        .map_err(storage)?;
                project["slug"].as_str().map(str::to_owned)
            }
            "unarchive" => {
                let id = id()?;
                unipi_kanboard::commands::project_set_archived(&self.layout, id, false)
                    .map_err(storage)?;
                Some(id.to_owned())
            }
            "enable_project_auto" => {
                let id = id()?;
                self.configure_project(id, true).await?;
                Some(id.to_owned())
            }
            "enable_auto_start" => {
                let current = self.transaction.lock().await.clone();
                self.configure(Config {
                    enabled: true,
                    ..current
                })
                .await?;
                None
            }
            _ => return Err(fail("E_SCHEMA", "Unknown enable action")),
        };
        Ok(json!({"ok":true,"projectId":result}))
    }

    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn blockers(&self, project_id: &str) -> Result<Vec<Blocker>> {
        let config = self.transaction.lock().await;
        let prefs = self.preferences.lock().await;
        let projects = self.layout.list_projects().map_err(storage)?;
        let project = projects.iter().find(|p| p.slug == project_id);
        Ok(project_blockers(
            project_id,
            project,
            config.enabled,
            prefs.projects.get(project_id).copied().unwrap_or(false),
        ))
    }

    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn configure_project_lead(
        &self,
        project_id: &str,
        model: Option<String>,
        thinking: Option<String>,
    ) -> Result<Value> {
        Project::load(&self.layout, project_id).map_err(storage)?;
        let normalize = |value: Option<String>| {
            value
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let model = normalize(model);
        let thinking = normalize(thinking);
        if model.as_ref().is_some_and(|model| {
            !model
                .split_once('/')
                .is_some_and(|(p, id)| !p.is_empty() && !id.is_empty())
        }) {
            return Err(fail("E_SCHEMA", "model must be provider/id"));
        }
        if let Some(thinking) = &thinking {
            thinking
                .parse::<oino_ai::ThinkingLevel>()
                .map_err(|_| fail("E_SCHEMA", "invalid thinking level"))?;
        }
        let mut prefs = self.preferences.lock().await;
        let mut next = prefs.clone();
        if model.is_none() && thinking.is_none() {
            next.project_leads.remove(project_id);
        } else {
            next.project_leads.insert(
                project_id.into(),
                ProjectLead {
                    model: model.clone(),
                    thinking: thinking.clone(),
                },
            );
        }
        next.save(&self.preferences_path).map_err(storage)?;
        *prefs = next;
        Ok(json!({"projectId":project_id,"model":model,"thinking":thinking}))
    }
    pub async fn presets_list(&self) -> Value {
        let prefs = self.preferences.lock().await;
        json!({"defaultPresetId":prefs.default_preset_id,"presets":prefs.presets})
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
    /// OIN-83 EXTRA: the Superagent sets a lead's model on the owner's behalf.
    /// Validates first (`E_UNKNOWN_MODEL`), switches the running lead through the
    /// same path the work pane uses, and with `remember` also saves it as the
    /// project's lead model so future leads keep it.
    ///
    /// Owner decision (not a defect): `remember: false` still persists the
    /// switched model on the live `LeadRecord` (inside `lead_action("switch_model")`)
    /// so the SAME lead chat keeps its model across host restarts; `remember`
    /// only governs whether the project preset for future leads is updated.
    /// Never invents a default: the model must be a real provider/id the lead's
    /// services know.
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn set_lead_model(
        &self,
        project_id: &str,
        model: &str,
        remember: bool,
    ) -> Result<Value> {
        let session_id = self
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .for_project(project_id)
            .map(|r| r.session_id.clone());
        let reply = if let Some(session_id) = session_id {
            crate::superagent_leads::lead_action(self, &session_id, "switch_model", Some(model))
                .await?
        } else {
            // No live lead: validate the model anyway so a typo fails now.
            let project = Project::load(&self.layout, project_id).map_err(storage)?;
            let (provider, id) = model
                .split_once('/')
                .filter(|(p, i)| !p.is_empty() && !i.is_empty())
                .ok_or_else(|| fail("E_SCHEMA", "model must be provider/id"))?;
            let services = self
                .workers
                .core_services(&project)
                .await
                .map_err(storage)?;
            if services.models.find(provider, id).is_none() {
                return Err(fail("E_UNKNOWN_MODEL", format!("Unknown model: {model}")));
            }
            json!({"ok": true, "switched": false})
        };
        if remember {
            let (provider, id) = model
                .split_once('/')
                .filter(|(p, i)| !p.is_empty() && !i.is_empty())
                .ok_or_else(|| fail("E_SCHEMA", "model must be provider/id"))?;
            let project = Project::load(&self.layout, project_id).map_err(storage)?;
            let services = self
                .workers
                .core_services(&project)
                .await
                .map_err(storage)?;
            if services.models.find(provider, id).is_none() {
                return Err(fail("E_UNKNOWN_MODEL", format!("Unknown model: {model}")));
            }
            self.configure_project_lead(project_id, Some(model.to_owned()), None)
                .await?;
        }
        Ok(
            json!({"ok": true, "projectId": project_id, "model": model, "remember": remember, "lead": reply}),
        )
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn set_mode(&self, project_id: &str, mode: &str) -> Result<Value> {
        if !matches!(mode, "none" | "goal" | "ralph" | "swarm" | "graph") {
            return Err(fail("E_SCHEMA", "Invalid long-horizon mode"));
        }
        let _transaction = self.transaction.lock().await;
        let id = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(_, p)| p.as_str() == project_id)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| fail("E_WORKER_CLAIM", "Project has no active lead"))?;
        let previous = self
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .for_project(project_id)
            .map(|preset| preset.preset.mode.clone())
            .or_else(|| {
                self.launch_presets
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&id)
                    .map(|preset| preset.mode.clone())
            });
        self.workers
            .set_mode(&id, mode)
            .await
            .map_err(|e| fail("E_MODE", e))?;
        tracing::info!(actor = "superagent", project = project_id, session = %id, from = previous.as_deref().unwrap_or("unknown"), to = mode, "lead mode changed");
        {
            let mut leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = leads.leads.get_mut(project_id) {
                record.preset.mode = mode.into();
                leads.save(&self.leads_path).map_err(storage)?;
            }
        }
        if let Some(preset) = self
            .launch_presets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(&id)
        {
            preset.mode = mode.into();
        }
        Ok(json!({"sessionId":id,"mode":mode}))
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn start(&self, project_id: &str, task_id: &str) -> Result<String> {
        self.start_with_preset(project_id, task_id, None, Overrides::default())
            .await
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn start_with_preset(
        &self,
        project_id: &str,
        task_id: &str,
        preset_id: Option<&str>,
        overrides: Overrides,
    ) -> Result<String> {
        self.start_tasks(project_id, &[task_id.to_owned()], preset_id, overrides)
            .await
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn start_tasks(
        &self,
        project_id: &str,
        task_ids: &[String],
        preset_id: Option<&str>,
        overrides: Overrides,
    ) -> Result<String> {
        self.start_tasks_with_lead(project_id, task_ids, preset_id, overrides, None)
            .await
    }
    /// `lead`: `auto` (default) wakes the project's lead chat, `new` starts a
    /// fresh one (retiring the current lead), `reuse` reopens the last stopped
    /// lead's chat (OIN-80).
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn start_tasks_with_lead(
        &self,
        project_id: &str,
        task_ids: &[String],
        preset_id: Option<&str>,
        overrides: Overrides,
        lead: Option<&str>,
    ) -> Result<String> {
        self.start_transaction(project_id, task_ids, false, preset_id, overrides, lead)
            .await
    }
    #[allow(
        clippy::too_many_lines,
        reason = "Dispatcher admission transaction must keep claim, recovery, preset, and rollback phases under the same guard."
    )]
    async fn start_transaction(
        &self,
        project_id: &str,
        task_ids: &[String],
        watcher: bool,
        preset_id: Option<&str>,
        overrides: Overrides,
        lead_mode: Option<&str>,
    ) -> Result<String> {
        let config = self.transaction.lock().await;
        if task_ids.is_empty()
            || task_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != task_ids.len()
        {
            return Err(fail("E_SCHEMA", "taskIds must be nonempty and unique"));
        }
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
        let requested = preset_id.is_some() || !overrides.is_empty();
        let mut overrides = overrides;
        if let Some(lead) = preferences.project_leads.get(project_id) {
            if overrides.model.is_none() {
                overrides.model.clone_from(&lead.model);
            }
            if overrides.thinking.is_none() {
                overrides.thinking.clone_from(&lead.thinking);
            }
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
        let project = self
            .layout
            .list_projects()
            .map_err(storage)?
            .into_iter()
            .find(|p| p.slug == project_id);
        let blockers = project_blockers(project_id, project.as_ref(), config.enabled, true);
        if let Some(blocker) = blockers.first() {
            let code = match blocker.code {
                "not_registered" => Some("E_PROJECT_NOT_REGISTERED"),
                "archived" => Some("E_PROJECT_ARCHIVED"),
                _ => None,
            };
            if let Some(code) = code {
                return Err(fail(code, &blocker.message));
            }
        }
        let project = project.expect("hard blockers reject missing projects");
        let record = self
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .for_project(project_id)
            .cloned();
        // OIN-80 wake (Q9): an existing record always re-opens the same chat —
        // no provider error, nudge budget or failed launch may mint a new id.
        // Only lead:"new" (below) or a stop/archive retires a lead.
        if let Some(record) = record {
            let id = record.session_id.clone();
            if lead_mode == Some("new") {
                crate::superagent_leads::retire_lead(self, project_id, "replaced").await?;
                // fall through to the fresh-launch path below with no record
            } else {
                if requested {
                    let running = Some(&record.preset);
                    let differs = running.map_or(preset.model.is_some(), |p| {
                        p.model != preset.model
                            || p.thinking != preset.thinking
                            || p.mode != preset.mode
                    });
                    if differs {
                        let (model, thinking) = running.map_or(("default model", "default"), |p| {
                            (
                                p.model.as_deref().unwrap_or("default model"),
                                p.thinking.as_deref().unwrap_or("default"),
                            )
                        });
                        let wanted = format!(
                            "{} ({})",
                            preset.model.as_deref().unwrap_or("default model"),
                            preset.thinking.as_deref().unwrap_or("default")
                        );
                        return Err(fail(
                            "E_LEAD_RUNNING",
                            format!(
                                "Project already has a running lead {id} on {model} ({thinking}); stop it with superagent_stop, then start again to use {wanted}"
                            ),
                        ));
                    }
                }
                // A live lead just gets the brief queued (the old occupied branch).
                let live = self.workers.live(&id).await;
                if live && self.workers.ended(&id).await.is_none() {
                    let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
                    let ids: Vec<String> = task_ids
                        .iter()
                        .filter(|task_id| {
                            !board.get(task_id).is_ok_and(|task| {
                                dispatcher::matches_claim(&task, &id)
                                    && (watcher || task.status != Status::Blocked)
                            })
                        })
                        .cloned()
                        .collect();
                    if ids.is_empty() {
                        return Ok(id.clone());
                    }
                    let tasks = if watcher {
                        dispatcher::reserve_many(&self.layout, project.clone(), &ids, &id)
                    } else {
                        dispatcher::reserve_many_manual(&self.layout, project.clone(), &ids, &id)
                    }
                    .map_err(|e| fail("E_WORKER_REFUSED", e))?;
                    let text = lead_brief(&project, &tasks, &record.preset);
                    for task in &tasks {
                        dispatcher::relay(
                            &self.layout,
                            project.clone(),
                            &task.id,
                            &id,
                            &text,
                            &uuid::Uuid::now_v7().to_string(),
                        )
                        .map_err(storage)?;
                    }
                    if let Err(e) = self
                        .workers
                        .relay_delivery(&id, &text, Delivery::Queue)
                        .await
                    {
                        for task in &tasks {
                            dispatcher::block(
                                &self.layout,
                                project.clone(),
                                &task.id,
                                &id,
                                &format!("Lead delivery failed: {e}; move to todo to retry"),
                            )
                            .map_err(storage)?;
                        }
                        return Err(fail("E_RELAY_DELIVERY", e));
                    }
                    return Ok(id.clone());
                }
                // Not live: wake the same chat (launch_lead attaches by id).
                let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
                let ids: Vec<String> = task_ids
                    .iter()
                    .filter(|task_id| {
                        !board.get(task_id).is_ok_and(|task| {
                            dispatcher::matches_claim(&task, &id)
                                && (watcher || task.status != Status::Blocked)
                        })
                    })
                    .cloned()
                    .collect();
                if ids.is_empty() {
                    self.register_lead_session(&id);
                    if let Err(e) = self
                        .workers
                        .launch_lead(&project, &[], &id, &record.preset)
                        .await
                    {
                        let now = crate::superagent_leads::now_ms();
                        crate::superagent_leads::enter_stall(
                            self,
                            &project,
                            crate::superagent_leads::HealthKind::Launch,
                            format!("Could not reopen lead: {e}"),
                            now,
                            Some(now + crate::superagent_leads::STALL_REPROBE_MS),
                        )
                        .await?;
                        return Err(fail("E_WORKER_LAUNCH", e));
                    }
                    let session_file = self.workers.session_file(&id).await;
                    {
                        let mut leads = self
                            .leads
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if let Some(record) = leads.leads.get_mut(project_id) {
                            record.state = crate::superagent_leads::LeadState::Working;
                            record.last_active_at = crate::superagent_leads::now_ms();
                            record.session_file = session_file;
                            leads.save(&self.leads_path).map_err(storage)?;
                        }
                    }
                    self.active
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(id.clone(), project_id.into());
                    return Ok(id);
                }
                let tasks = if watcher {
                    dispatcher::reserve_many(&self.layout, project.clone(), &ids, &id)
                } else {
                    dispatcher::reserve_many_manual(&self.layout, project.clone(), &ids, &id)
                }
                .map_err(|e| fail("E_WORKER_REFUSED", e))?;
                let text = lead_brief(&project, &tasks, &record.preset);
                for task in &tasks {
                    dispatcher::relay(
                        &self.layout,
                        project.clone(),
                        &task.id,
                        &id,
                        &text,
                        &uuid::Uuid::now_v7().to_string(),
                    )
                    .map_err(storage)?;
                }
                self.register_lead_session(&id);
                if let Err(e) = self
                    .workers
                    .launch_lead(&project, &tasks, &id, &record.preset)
                    .await
                {
                    // A failed launch never blocks the tickets into a new chat:
                    // the record stalls with Launch and keeps its claims (OIN-80).
                    let now = crate::superagent_leads::now_ms();
                    crate::superagent_leads::enter_stall(
                        self,
                        &project,
                        crate::superagent_leads::HealthKind::Launch,
                        format!("Could not reopen lead: {e}"),
                        now,
                        Some(now + crate::superagent_leads::STALL_REPROBE_MS),
                    )
                    .await?;
                    return Err(fail("E_WORKER_LAUNCH", e));
                }
                let session_file = self.workers.session_file(&id).await;
                {
                    let mut leads = self
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(record) = leads.leads.get_mut(project_id) {
                        record.state = crate::superagent_leads::LeadState::Working;
                        record.last_active_at = crate::superagent_leads::now_ms();
                        record.session_file = session_file;
                        leads.save(&self.leads_path).map_err(storage)?;
                    }
                }
                self.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id.clone(), project_id.into());
                if let Err(e) = self
                    .workers
                    .relay_delivery(&id, &text, Delivery::Queue)
                    .await
                {
                    for task in &tasks {
                        dispatcher::block(
                            &self.layout,
                            project.clone(),
                            &task.id,
                            &id,
                            &format!("Lead delivery failed: {e}; move to todo to retry"),
                        )
                        .map_err(storage)?;
                    }
                    return Err(fail("E_RELAY_DELIVERY", e));
                }
                return Ok(id);
            }
        }
        // lead:"reuse": reopen the last retired lead whose session file still exists.
        if lead_mode == Some("reuse") {
            let reuse = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retired
                .iter()
                .rev()
                .find(|entry| {
                    entry.project_id == project_id
                        && entry
                            .session_file
                            .as_ref()
                            .is_some_and(|file| file.exists())
                })
                .map(|entry| (entry.session_id.clone(), entry.session_file.clone()));
            if let Some((old_id, fallback_file)) = reuse {
                let tasks = if watcher {
                    dispatcher::reserve_many(&self.layout, project.clone(), task_ids, &old_id)
                } else {
                    dispatcher::reserve_many_manual(
                        &self.layout,
                        project.clone(),
                        task_ids,
                        &old_id,
                    )
                }
                .map_err(|e| fail("E_WORKER_REFUSED", e))?;
                // N3: persist the current LeadRecord for `old_id` BEFORE registering or
                // launching the runtime. On save failure, roll back reservations and
                // return with nothing launched/registered so `old_id` remains reusable.
                let now = crate::superagent_leads::now_ms();
                let save_result = {
                    let mut leads = self
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut next = leads.clone();
                    next.retired.retain(|r| r.session_id != old_id);
                    next.leads.insert(
                        project_id.to_owned(),
                        crate::superagent_leads::LeadRecord {
                            project_id: project_id.to_owned(),
                            session_id: old_id.clone(),
                            session_file: fallback_file.clone(),
                            preset: preset.clone(),
                            state: crate::superagent_leads::LeadState::Working,
                            created_at: now,
                            last_active_at: now,
                            health: None,
                            continuation: crate::superagent_leads::Continuation {
                                tickets: tasks.iter().map(|t| t.id.clone()).collect(),
                                sent: false,
                                total: 0,
                            },
                            question: None,
                            pending_answers: Vec::new(),
                            on_fallback: false,
                            resume_sent_at: None,
                            in_progress_tickets: Vec::new(),
                        },
                    );
                    next.save(&self.leads_path).map(|()| {
                        *leads = next;
                    })
                };
                if let Err(save_err) = save_result {
                    if let Ok(_lock) = self.layout.lock_board(&project.slug) {
                        if let Ok(board) = Board::open(&self.layout, project.clone()) {
                            for task in &tasks {
                                if let Ok(mut current) = board.get(&task.id) {
                                    if dispatcher::matches_claim(&current, &old_id) {
                                        current.status = Status::Todo;
                                        current.run = None;
                                        let _ = board.save(&current);
                                    }
                                }
                            }
                        }
                    }
                    return Err(save_err);
                }
                let text = lead_brief(&project, &tasks, &preset);
                for task in &tasks {
                    dispatcher::relay(
                        &self.layout,
                        project.clone(),
                        &task.id,
                        &old_id,
                        &text,
                        &uuid::Uuid::now_v7().to_string(),
                    )
                    .map_err(storage)?;
                }
                // Item 7: register in `lead_sessions` BEFORE launching so turn-end
                // Settled events stay suppressed on reused leads.
                self.register_lead_session(&old_id);
                if let Err(e) = self
                    .workers
                    .launch_lead(&project, &tasks, &old_id, &preset)
                    .await
                {
                    // N3: launch failure keeps `old_id` as the project's stalled lead
                    // (claims kept, never fresh allocation on a subsequent start).
                    let stall_now = crate::superagent_leads::now_ms();
                    crate::superagent_leads::enter_stall(
                        self,
                        &project,
                        crate::superagent_leads::HealthKind::Launch,
                        format!("Could not reopen lead: {e}"),
                        stall_now,
                        Some(stall_now + crate::superagent_leads::STALL_REPROBE_MS),
                    )
                    .await?;
                    return Err(fail("E_WORKER_LAUNCH", e));
                }
                let reused_file = self.workers.session_file(&old_id).await.or(fallback_file);
                {
                    let mut leads = self
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let Some(record) = leads.leads.get_mut(project_id) {
                        record.session_file = reused_file;
                        record.last_active_at = crate::superagent_leads::now_ms();
                        leads.save(&self.leads_path).map_err(storage)?;
                    }
                }
                self.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(old_id.clone(), project_id.into());
                if let Err(e) = self
                    .workers
                    .relay_delivery(&old_id, &text, Delivery::Queue)
                    .await
                {
                    for task in &tasks {
                        dispatcher::block(
                            &self.layout,
                            project.clone(),
                            &task.id,
                            &old_id,
                            &format!("Lead delivery failed: {e}; move to todo to retry"),
                        )
                        .map_err(storage)?;
                    }
                    return Err(fail("E_RELAY_DELIVERY", e));
                }
                return Ok(old_id);
            }
        }
        // TODO(OIN-56 phase 2): when config.paused_graph_policy == PausedGraphPolicy::Reuse and a parked graph owner exists for this project, wake/reuse that previous lead session instead of launching a new Plain lead.
        let id = uuid::Uuid::now_v7().to_string();
        let tasks = if watcher {
            dispatcher::reserve_many(&self.layout, project.clone(), task_ids, &id)
        } else {
            dispatcher::reserve_many_manual(&self.layout, project.clone(), task_ids, &id)
        }
        .map_err(|e| fail("E_WORKER_REFUSED", e))?;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), project_id.into());
        // OIN-81 / Item 7: register in `lead_sessions` BEFORE runtime runs.
        self.register_lead_session(&id);
        // Persisted BEFORE the launch: a crash mid-launch (process death) leaves
        // a Working record, so the next acquire resumes the same id instead of
        // parking the claims with INTERRUPTED.
        {
            let mut leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads.leads.insert(
                project_id.to_owned(),
                crate::superagent_leads::LeadRecord {
                    project_id: project_id.to_owned(),
                    session_id: id.clone(),
                    session_file: None,
                    preset: preset.clone(),
                    state: crate::superagent_leads::LeadState::Working,
                    created_at: crate::superagent_leads::now_ms(),
                    last_active_at: crate::superagent_leads::now_ms(),
                    health: None,
                    continuation: crate::superagent_leads::Continuation {
                        tickets: tasks.iter().map(|t| t.id.clone()).collect(),
                        sent: false,
                        total: 0,
                    },
                    question: None,
                    pending_answers: Vec::new(),
                    on_fallback: false,
                    resume_sent_at: None,
                    in_progress_tickets: Vec::new(),
                },
            );
            leads.save(&self.leads_path).map_err(storage)?;
        }
        if let Err(e) = self
            .workers
            .launch_lead(&project, &tasks, &id, &preset)
            .await
        {
            // OIN-80 / Item 8: a failed launch stalls the record (Launch health),
            // keeps the claims, and routes through deduplicated `enter_stall`
            // (setting `notified = true` only after sending the notice).
            self.active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            let now = crate::superagent_leads::now_ms();
            crate::superagent_leads::enter_stall(
                self,
                &project,
                crate::superagent_leads::HealthKind::Launch,
                format!("Could not start lead: {e}"),
                now,
                Some(now + crate::superagent_leads::STALL_REPROBE_MS),
            )
            .await?;
            return Err(fail("E_WORKER_LAUNCH", e));
        }
        {
            let session_file = self.workers.session_file(&id).await;
            let mut leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leads.leads.insert(
                project_id.to_owned(),
                crate::superagent_leads::LeadRecord {
                    project_id: project_id.to_owned(),
                    session_id: id.clone(),
                    session_file,
                    preset: preset.clone(),
                    state: crate::superagent_leads::LeadState::Working,
                    created_at: crate::superagent_leads::now_ms(),
                    last_active_at: crate::superagent_leads::now_ms(),
                    health: None,
                    continuation: crate::superagent_leads::Continuation {
                        tickets: tasks.iter().map(|t| t.id.clone()).collect(),
                        sent: false,
                        total: 0,
                    },
                    question: None,
                    pending_answers: Vec::new(),
                    on_fallback: false,
                    resume_sent_at: None,
                    in_progress_tickets: Vec::new(),
                },
            );
            leads.save(&self.leads_path).map_err(storage)?;
        }
        self.launch_presets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), preset);
        Ok(id)
    }
    /// Reap completed workers, then offer ready todo tickets through start's transaction.
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn tick(&self) -> Result<()> {
        self.tick_at(crate::superagent_leads::now_ms()).await
    }
    /// `tick` at an explicit time (tests drive the lead machine's probes and
    /// warm-unloads through this without sleeping).
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn tick_at(&self, now: i64) -> Result<()> {
        {
            let _transaction = self.transaction.lock().await;
            // OIN-80: the lead machine replaced the reaping loop. Errors stall,
            // nudges budget, unloads keep the record — never a new session id.
            crate::superagent_leads::lead_tick(self, now).await?;
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
            let ids: Vec<String> = tasks
                .iter()
                .filter(|task| {
                    unipi_kanboard::deps::is_ready(
                        task,
                        &board.dep_lookup(&tasks),
                        unipi_kanboard::model::ChainGate::InReview,
                    )
                })
                .map(|task| task.id.clone())
                .collect();
            if !ids.is_empty() {
                let _ = self
                    .start_transaction(&project.slug, &ids, true, None, Overrides::default(), None)
                    .await;
            }
        }
        Ok(())
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
        // OIN-80: a stop retires the lead (both in_progress and blocked tickets
        // move to backlog with `Released incomplete:` so Auto-start does not relaunch them).
        if self
            .leads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .for_project(project_id)
            .is_some_and(|record| record.session_id == run.session)
        {
            crate::superagent_leads::retire_lead(self, project_id, "stopped").await?;
            return Ok(true);
        }
        for (claimed_project, claimed) in self.claims()? {
            if claimed_project.slug == project_id
                && claimed
                    .run
                    .as_ref()
                    .is_some_and(|r| r.session == run.session)
            {
                dispatcher::block(
                    &self.layout,
                    claimed_project,
                    &claimed.id,
                    &run.session,
                    "Worker stopped; move to todo to retry",
                )
                .map_err(storage)?;
            }
        }
        Ok(true)
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn relay(&self, project_id: &str, task_id: &str, text: &str) -> Result<Value> {
        self.relay_to_lead(
            project_id,
            text,
            &[task_id.to_owned()],
            Delivery::Queue,
            false,
        )
        .await
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn relay_to_lead(
        &self,
        project_id: &str,
        text: &str,
        task_ids: &[String],
        delivery: Delivery,
        priority: bool,
    ) -> Result<Value> {
        self.relay_to_lead_checked(project_id, text, task_ids, delivery, priority, None, None)
            .await
    }
    /// Deliver a blocked-question answer to `id` FIRST (waking `id` if unloaded),
    /// and ONLY after delivery succeeds record `Owner answer:` and move `task_id`
    /// `blocked -> in_progress` under `id`'s claim (Item 2). Caller must hold
    /// `self.transaction`.
    async fn deliver_and_resume_blocked_locked(
        &self,
        project: &Project,
        task_id: &str,
        id: &str,
        text: &str,
    ) -> Result<Value> {
        let now = crate::superagent_leads::now_ms();
        let task = Board::open(&self.layout, project.clone())
            .map_err(storage)?
            .get(task_id)
            .map_err(storage)?;
        let question = task
            .activity
            .iter()
            .rev()
            .find(|a| a.text.starts_with("blocked:"))
            .map(|a| a.text.trim_start_matches("blocked:").trim().to_owned())
            .unwrap_or_default();
        let trimmed = text.trim();
        let delivery_text =
            format!("Owner answer on {task_id}: {trimmed}\n\nYour question was: {question}");
        // N1 + F2: stage a per-operation outbox entry for `task_id` BEFORE delivery.
        // If an unfinished `BlockedAnswer` entry for `task_id` already exists:
        // - when `delivered == true`, keep `delivered: true` so retry finishes only board resume;
        // - when `delivered == false`, update its text/delivery_text in place.
        // Never touch or flush another ticket's outbox entry (F2).
        let entry_id = {
            let mut leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut next = leads.clone();
            let record = next
                .leads
                .get_mut(&project.slug)
                .filter(|r| r.session_id == id)
                .ok_or_else(|| {
                    fail(
                        "E_LEAD_UNKNOWN",
                        "No lead with this id; refresh the work pane",
                    )
                })?;
            let entry_id = if let Some(existing) = record.pending_answers.iter_mut().find(|pa| {
                pa.kind == crate::superagent_leads::PendingAnswerKind::BlockedAnswer
                    && pa.task_id == task_id
            }) {
                if !existing.delivered {
                    trimmed.clone_into(&mut existing.text);
                    existing.delivery_text = delivery_text;
                }
                existing.id.clone()
            } else {
                let new_id = uuid::Uuid::now_v7().to_string();
                record
                    .pending_answers
                    .push(crate::superagent_leads::PendingAnswer {
                        id: new_id.clone(),
                        kind: crate::superagent_leads::PendingAnswerKind::BlockedAnswer,
                        task_id: task_id.to_owned(),
                        request_id: None,
                        dialog_answer: None,
                        question_excerpt: None,
                        text: trimmed.to_owned(),
                        delivery_text,
                        mode: crate::superagent_leads::PendingAnswerMode::Relay,
                        dialog_closed: true,
                        delivered: false,
                        board_done: false,
                        attempts: 0,
                        next_attempt_at: None,
                        runtime_failures: 0,
                        created_at: now,
                    });
                new_id
            };
            next.save(&self.leads_path).map_err(storage)?;
            *leads = next;
            entry_id
        };
        crate::superagent_leads::flush_pending_answers(
            self,
            project,
            now,
            Some(crate::superagent_leads::OutboxFilter::EntryId(entry_id)),
            true,
            true,
        )
        .await?;
        Ok(json!({"delivered":true,"answered":"blocked","sessionId":id}))
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "checked relay carries delivery and correlation fields"
    )]
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    /// # Panics
    /// Panics if a previously checked active lead identifier unexpectedly becomes absent.
    #[allow(
        clippy::too_many_lines,
        reason = "Relay state machine retains expected-claim checks and durable outbox steps in their existing transaction order."
    )]
    pub async fn relay_to_lead_checked(
        &self,
        project_id: &str,
        text: &str,
        task_ids: &[String],
        delivery: Delivery,
        priority: bool,
        expected_session: Option<&str>,
        expected_request_id: Option<&str>,
    ) -> Result<Value> {
        let transaction = self.transaction.lock().await;
        if text.trim().is_empty() {
            return Err(fail("E_SCHEMA", "Relay text must not be empty"));
        }
        let project = Project::load(&self.layout, project_id).map_err(storage)?;
        // Item 5: when the caller carries an expected session / request id, re-check
        // under `self.transaction` that the session is still the project's current lead,
        // still claims every targeted ticket, and matches any open question id.
        if let Some(expected_sid) = expected_session.map(str::trim).filter(|s| !s.is_empty()) {
            let current_sid = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .for_project(project_id)
                .map(|r| r.session_id.clone());
            if current_sid.as_deref() != Some(expected_sid) {
                return Err(fail(
                    "E_LEAD_UNKNOWN",
                    "Lead session changed; refresh the work pane",
                ));
            }
            let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
            for tid in task_ids {
                let task = board.get(tid).map_err(storage)?;
                if !dispatcher::matches_claim(&task, expected_sid) {
                    return Err(fail(
                        "E_LEAD_UNKNOWN",
                        "Ticket claim no longer matches this lead; refresh the work pane",
                    ));
                }
            }
        }
        if let Some(expected_req) = expected_request_id.map(str::trim).filter(|s| !s.is_empty()) {
            let matches_open_or_outbox = {
                let leads = self
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                leads.for_project(project_id).is_some_and(|r| {
                    let open_matches = r
                        .question
                        .as_ref()
                        .filter(|q| !(q.timing_out && q.board_blocked))
                        .is_some_and(|q| q.request_id == expected_req);
                    let outbox_matches = r.pending_answers.iter().any(|pa| {
                        pa.kind == crate::superagent_leads::PendingAnswerKind::OpenQuestion
                            && pa.request_id.as_deref() == Some(expected_req)
                    });
                    open_matches || outbox_matches
                })
            };
            if !matches_open_or_outbox {
                return Err(fail(
                    "E_ALREADY_ANSWERED",
                    "This question was already answered.",
                ));
            }
        }
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(_, p)| p.as_str() == project_id)
            .map(|(id, _)| id.clone())
            // An unloaded/unloaded-after-restart lead has no `active` entry but its
            // record does: wake it instead of failing (OIN-80 wake, OIN-81 !live).
            .or_else(|| {
                self.leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .for_project(project_id)
                    .map(|r| r.session_id.clone())
            });
        let live = if let Some(id) = &active {
            self.workers.live(id).await && self.workers.ended(id).await.is_none()
        } else {
            false
        };
        if task_ids.len() == 1 {
            let task_id = &task_ids[0];
            let has_record = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .for_project(project_id)
                .is_some();
            // N2: if an `OpenQuestion` outbox entry for this task/request is already
            // staged (e.g. dialog closed on the previous attempt, steer failed),
            // resume from the missing step without answering the closed dialog again
            // — even if the runtime went idle/offline meanwhile.
            let pending_open_id = {
                let leads = self
                    .leads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                leads.for_project(project_id).and_then(|r| {
                    r.pending_answers
                        .iter()
                        .find(|pa| {
                            pa.kind == crate::superagent_leads::PendingAnswerKind::OpenQuestion
                                && pa.task_id == *task_id
                                && expected_request_id
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .is_none_or(|req| pa.request_id.as_deref() == Some(req))
                        })
                        .map(|pa| pa.id.clone())
                })
            };
            if let Some(entry_id) = pending_open_id {
                let id = active.clone().expect("record present");
                let now = crate::superagent_leads::now_ms();
                crate::superagent_leads::flush_pending_answers(
                    self,
                    &project,
                    now,
                    Some(crate::superagent_leads::OutboxFilter::EntryId(entry_id)),
                    true,
                    true,
                )
                .await?;
                return Ok(json!({"delivered":true,"answered":"question","sessionId":id}));
            }
            // Branch (2) resumes the ticket on the SAME session id only for a ticket
            // the lead itself holds blocked under its claim (the timeout-converted
            // question, OIN-81) — whether the lead is currently unloaded (!live) or
            // still live working its other tickets. A manually blocked, unclaimed
            // ticket keeps the legacy unblock+restart path (also archived projects).
            let claimed_blocked = Board::open(&self.layout, project.clone())
                .map_err(storage)?
                .get(task_id)
                .map_err(storage)
                .is_ok_and(|task| {
                    task.status == Status::Blocked
                        && active
                            .as_ref()
                            .is_some_and(|id| dispatcher::matches_claim(&task, id))
                });
            if has_record && claimed_blocked {
                let id = active.clone().expect("record present");
                return self
                    .deliver_and_resume_blocked_locked(&project, task_id, &id, text)
                    .await;
            }
            if !live {
                {
                    let _lock = self.layout.lock_board(&project.slug).map_err(storage)?;
                    let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
                    let mut task = board.get(task_id).map_err(storage)?;
                    if task.status != Status::Blocked {
                        return Err(fail("E_WORKER_CLAIM", "Ticket is no longer blocked"));
                    }
                    let now = chrono::Utc::now();
                    if let Some(prev_run) = task.run.take() {
                        let prev_question =
                            dispatcher::last_blocked_question(&task).map(str::to_owned);
                        task.push_activity(
                            now,
                            unipi_kanboard::model::Actor::System,
                            dispatcher::released_incomplete_line(
                                Status::Blocked,
                                &prev_run.session,
                                "worker ended while blocked",
                                prev_question.as_deref(),
                            ),
                        );
                    }
                    task.status = Status::Todo;
                    task.push_activity(
                        now,
                        unipi_kanboard::model::Actor::User,
                        format!("Owner answer: {text}"),
                    );
                    board.save(&task).map_err(storage)?;
                }
                if let Some(id) = active {
                    self.active
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&id);
                }
                drop(transaction);
                return match self.start(project_id, task_id).await {
                    Ok(id) => Ok(json!({"delivered":true,"started":true,"sessionId":id})),
                    Err(e) => {
                        if e.code == "E_WORKER_LAUNCH" {
                            let _lock = self.layout.lock_board(&project.slug).map_err(storage)?;
                            let board = Board::open(&self.layout, project).map_err(storage)?;
                            let mut task = board.get(task_id).map_err(storage)?;
                            if task.status == Status::Blocked
                                && task.run.is_none()
                                && task
                                    .activity
                                    .iter()
                                    .rev()
                                    .any(|a| a.text.starts_with("blocked: Worker could not start:"))
                            {
                                task.status = Status::Todo;
                                board.save(&task).map_err(storage)?;
                            }
                        }
                        Ok(
                            json!({"delivered":false,"saved":true,"error":{"code":e.code,"message":format!("Answer saved, but could not start a lead: {}", e.message)}}),
                        )
                    }
                };
            }
        }
        let id = active.ok_or_else(|| fail("E_WORKER_CLAIM", "Project has no active lead"))?;
        if !live {
            return Err(fail(
                "E_WORKER_NOT_LIVE",
                "Lead has ended; refresh the board before relaying",
            ));
        }
        // OIN-81 branch (1): a single ticket whose task matches the lead's OPEN
        // question answers the dialog directly (same turn, first-wins).
        if task_ids.len() == 1 {
            let task_id = &task_ids[0];
            let question = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .for_project(project_id)
                .and_then(|record| record.question.clone())
                .filter(|question| {
                    question.task_id == *task_id && !(question.timing_out && question.board_blocked)
                });
            let already = question.is_none() && {
                let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
                let task = board.get(task_id).map_err(storage)?;
                task.status == Status::InProgress
                    && dispatcher::matches_claim(&task, &id)
                    && task.activity.iter().any(|a| {
                        a.actor == unipi_kanboard::model::Actor::User
                            && a.text.starts_with("Owner answer:")
                    })
            };
            if already {
                return Err(fail(
                    "E_ALREADY_ANSWERED",
                    "This question was already answered.",
                ));
            }
            if let Some(question) = question {
                let task = Board::open(&self.layout, project.clone())
                    .map_err(storage)?
                    .get(task_id)
                    .map_err(storage)?;
                if !dispatcher::matches_claim(&task, &id) {
                    return Err(fail(
                        "E_LEAD_UNKNOWN",
                        "Ticket claim no longer matches this lead; refresh the work pane",
                    ));
                }
                let live_dialog = self
                    .workers
                    .questions(&id)
                    .await
                    .into_iter()
                    .find(|q| q.request_id == question.request_id);
                let Some(live_dialog) = live_dialog else {
                    return Err(fail(
                        "E_ALREADY_ANSWERED",
                        "This question was already answered.",
                    ));
                };
                let now = crate::superagent_leads::now_ms();
                let encoded = crate::superagent_questions::encode_owner_answer(
                    Some(&live_dialog),
                    &question.excerpt,
                    text,
                );
                let trimmed = text.trim();
                let dialog_answer =
                    crate::superagent_leads::ui_answer_to_value(&encoded.dialog_answer);
                // N2 + F1 + F2: durably stage the owner's verbatim words AND exact encoded
                // `dialog_answer` (+ `question_excerpt`) BEFORE closing the dialog, with
                // `delivered: false` until either `workers.answer` or steer fallback succeeds.
                let entry_id = uuid::Uuid::now_v7().to_string();
                {
                    let mut leads = self
                        .leads
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut next = leads.clone();
                    if let Some(record) = next.leads.get_mut(project_id) {
                        record
                            .pending_answers
                            .push(crate::superagent_leads::PendingAnswer {
                                id: entry_id.clone(),
                                kind: crate::superagent_leads::PendingAnswerKind::OpenQuestion,
                                task_id: task_id.clone(),
                                request_id: Some(question.request_id.clone()),
                                dialog_answer: Some(dialog_answer),
                                question_excerpt: Some(question.excerpt.clone()),
                                text: trimmed.to_owned(),
                                delivery_text: encoded.steer_message.clone().unwrap_or_default(),
                                mode: crate::superagent_leads::PendingAnswerMode::Steer,
                                dialog_closed: false,
                                delivered: false,
                                board_done: false,
                                attempts: 0,
                                next_attempt_at: None,
                                runtime_failures: 0,
                                created_at: now,
                            });
                        next.save(&self.leads_path).map_err(storage)?;
                        *leads = next;
                    }
                }
                crate::superagent_leads::flush_pending_answers(
                    self,
                    &project,
                    now,
                    Some(crate::superagent_leads::OutboxFilter::EntryId(entry_id)),
                    true,
                    true,
                )
                .await?;
                return Ok(json!({"delivered":true,"answered":"question","sessionId":id}));
            }
        }
        let board = Board::open(&self.layout, project.clone()).map_err(storage)?;
        let ids: Vec<String> = if task_ids.is_empty() {
            board
                .tasks()
                .map_err(storage)?
                .into_iter()
                .filter(|t| dispatcher::matches_claim(t, &id))
                .map(|t| t.id)
                .collect()
        } else {
            task_ids.to_vec()
        };
        let text = if priority {
            format!("URGENT: schedule this first / add it as the first graph item\n\n{text}")
        } else {
            text.to_owned()
        };
        let comment = uuid::Uuid::now_v7().to_string();
        for task_id in &ids {
            let task = board.get(task_id).map_err(storage)?;
            if task.status == Status::Blocked && task.run.is_none() {
                let _lock = self.layout.lock_board(&project.slug).map_err(storage)?;
                let mut task = board.get(task_id).map_err(storage)?;
                if task.status != Status::Blocked || task.run.is_some() {
                    return Err(fail(
                        "E_WORKER_CLAIM",
                        "Ticket changed; refresh before relaying",
                    ));
                }
                task.push_activity(
                    chrono::Utc::now(),
                    unipi_kanboard::model::Actor::User,
                    format!("Owner answer: {text}"),
                );
                task.status = Status::Todo;
                board.save(&task).map_err(storage)?;
            } else {
                dispatcher::relay(&self.layout, project.clone(), task_id, &id, &text, &comment)
                    .map_err(storage)?;
            }
        }
        let mut reply =
            json!({"commentId": if ids.is_empty() { None } else { Some(comment) }, "sessionId":id});
        match self.workers.relay_delivery(&id, &text, delivery).await {
            Ok(()) => reply["delivered"] = json!(true),
            Err(e) => {
                reply["delivered"] = json!(false);
                reply["error"] = json!({"code":"E_RELAY_DELIVERY", "message":format!("Comment saved, but delivery failed: {e}")});
            }
        }
        Ok(reply)
    }
    /// OIN-81 `superagent.answer` RPC: the work pane's answer to a lead's
    /// BLOCKED question (the timeout conversion). Validates under
    /// `self.transaction` that `session_id` is still the project's current lead
    /// and the ticket's claimant, delivers to `session_id` FIRST, and only on
    /// delivery success moves the ticket `blocked -> in_progress` (Items 2 & 5).
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub async fn answer_blocked_question(
        &self,
        session_id: &str,
        task_id: &str,
        answer: &str,
    ) -> Result<Value> {
        if answer.trim().is_empty() {
            return Err(fail("E_SCHEMA", "Answer text must not be empty"));
        }
        let session = session_id.trim();
        if session.is_empty() {
            return Err(fail(
                "E_LEAD_UNKNOWN",
                "No lead with this id; refresh the work pane",
            ));
        }
        let _transaction = self.transaction.lock().await;
        let project_id = {
            let leads = self
                .leads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let record = leads
                .leads
                .values()
                .find(|record| record.session_id == session)
                .filter(|record| {
                    leads
                        .for_project(&record.project_id)
                        .is_some_and(|current| current.session_id == session)
                })
                .ok_or_else(|| {
                    fail(
                        "E_LEAD_UNKNOWN",
                        "No lead with this id; refresh the work pane",
                    )
                })?;
            record.project_id.clone()
        };
        let project = Project::load(&self.layout, &project_id).map_err(storage)?;
        let task = Board::open(&self.layout, project.clone())
            .map_err(storage)?
            .get(task_id)
            .map_err(storage)?;
        if task.status != Status::Blocked {
            if task.run.as_ref().is_some_and(|r| r.session != session) {
                return Err(fail(
                    "E_LEAD_UNKNOWN",
                    "Ticket claim moved to another session; refresh the work pane",
                ));
            }
            return Err(fail(
                "E_ALREADY_ANSWERED",
                format!(
                    "{task_id} is {}, not blocked — the question was already answered or the ticket moved on",
                    task.status
                ),
            ));
        }
        if !dispatcher::matches_claim(&task, session) {
            return Err(fail(
                "E_LEAD_UNKNOWN",
                "Ticket claim no longer matches this lead; refresh the work pane",
            ));
        }
        self.deliver_and_resume_blocked_locked(&project, task_id, session, answer)
            .await
    }
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    pub fn append_recent_workspaces(
        &self,
        status: &mut Value,
        recent: &[crate::sessions::card::ProjectGroup],
    ) -> Result<()> {
        let registered = self.layout.list_projects().map_err(storage)?;
        let Some(rows) = status["projects"].as_array_mut() else {
            return Ok(());
        };
        let mut added = 0;
        for workspace in recent {
            let path = Path::new(&workspace.cwd);
            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !path.is_absolute()
                || registered.iter().any(|project| {
                    project.root == canonical
                        || std::fs::canonicalize(&project.root).is_ok_and(|root| root == canonical)
                })
            {
                continue;
            }
            let slug = unipi_kanboard::store::slug_for(&canonical);
            if rows.iter().any(|row| row["projectId"] == slug) {
                continue;
            }
            let blockers = project_blockers(&workspace.cwd, None, false, false);
            rows.push(json!({"projectId":slug,"name":workspace.name,"path":workspace.cwd,"registered":false,"autoStart":false,"lead":null,"ready":0,"blockers":blockers}));
            added += 1;
            if added == 5 {
                break;
            }
        }
        Ok(())
    }

    /// # Errors
    /// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
    #[allow(
        clippy::too_many_lines,
        reason = "Read-only status join keeps board, worker, lead, and question projections in one snapshot pass."
    )]
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
            let ready = tasks
                .iter()
                .filter(|task| {
                    unipi_kanboard::deps::is_ready(
                        task,
                        &board.dep_lookup(&tasks),
                        unipi_kanboard::model::ChainGate::InReview,
                    )
                })
                .count();
            for task in tasks {
                if task.status == Status::Archived {
                    continue;
                }
                if let Some(run) = &task.run {
                    if run.owner == RunOwner::Dispatcher {
                        let progress = self.workers.progress(&run.session).await;
                        let preset = self
                            .leads
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .for_project(&project.slug)
                            .filter(|record| record.session_id == run.session)
                            .map(|record| record.preset.clone())
                            .or_else(|| {
                                self.launch_presets
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .get(&run.session)
                                    .cloned()
                            });
                        if self.workers.live(&run.session).await
                            && self.workers.ended(&run.session).await.is_none()
                        {
                            lead = Some(run.session.clone());
                            if let Some(existing) = leads
                                .iter_mut()
                                .find(|entry: &&mut Value| entry["sessionId"] == run.session)
                            {
                                if let Some(ids) = existing["ticketIds"].as_array_mut() {
                                    ids.push(json!(task.id));
                                }
                                existing["progress"]["running"] =
                                    json!(existing["ticketIds"].as_array().map_or(0, Vec::len));
                            } else {
                                let paused_graph = self.workers.agent_dir().and_then(|dir| {
                                    paused_graph_for_lead(dir, &project.root, &run.session)
                                });
                                let mut entry = json!({"projectId":project.slug,"sessionId":run.session,"presetId":preset.as_ref().map(|p| &p.id),"model":progress.get("model").filter(|v| !v.is_null()).cloned().or_else(|| preset.as_ref().and_then(|p| p.model.as_ref()).map(|v| json!(v))),"thinking":progress.get("thinking").filter(|v| !v.is_null()).cloned().or_else(|| preset.as_ref().and_then(|p| p.thinking.as_ref()).map(|v| json!(v))),"autonomy":progress.get("autonomy").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(preset.as_ref().map_or("high", |p| &p.autonomy))),"mode":preset.as_ref().map_or("none", |p| &p.mode),"ticketIds":[task.id],"step":progress["step"],"tool":progress["tool"],"startedAt":run.started,"cost":progress["cost"],"progress":{"running":1,"queued":0,"done":0}});
                                if let Some(paused_graph) = paused_graph {
                                    entry["pausedGraph"] = json!(paused_graph);
                                }
                                leads.push(entry);
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
                } else if task.status == Status::Blocked {
                    if let Some(run) = &task.run {
                        // OIN-81: a blocked ticket whose claim the lead kept is the
                        // timeout-converted question; the answer wakes the same
                        // session. Skipped when an approval card already covers it.
                        if dispatcher::matches_claim(&task, &run.session)
                            && !needs_you
                                .iter()
                                .any(|row: &Value| row["taskId"] == json!(task.id))
                        {
                            if let Some(question) = task
                                .activity
                                .iter()
                                .rev()
                                .find(|a| a.text.starts_with("blocked:"))
                                .map(|a| a.text.trim_start_matches("blocked: "))
                                .filter(|q| !q.trim().is_empty())
                            {
                                needs_you.push(crate::superagent_questions::blocked_row(
                                    &project.slug,
                                    &task.id,
                                    &run.session,
                                    question,
                                ));
                            }
                        }
                    } else if let Some(question) = task
                        .activity
                        .iter()
                        .rev()
                        .find(|a| a.text.starts_with("blocked:"))
                        .map(|a| a.text.trim_start_matches("blocked: "))
                        .filter(|q| !q.trim().is_empty())
                        // Plumbing texts (OIN-80 migration, F7) never surface.
                        .filter(|q| !crate::superagent_questions::is_plumbing(q))
                    {
                        needs_you.push(json!({"kind":"question","projectId":project.slug,"taskId":task.id,"status":"blocked","question":question}));
                    }
                }
            }
            let auto_start = prefs.projects.get(&project.slug).copied().unwrap_or(false);
            let project_lead = prefs.project_leads.get(&project.slug);
            let blockers =
                project_blockers(&project.slug, Some(&project), config.enabled, auto_start);
            projects.push(json!({"projectId":project.slug,"name":project.name,"autoStart":auto_start,"lead":lead,"ready":ready,"blockers":blockers,"registered":true,"leadModel":project_lead.and_then(|lead| lead.model.clone()),"leadThinking":project_lead.and_then(|lead| lead.thinking.clone()),"root":project.root}));
        }
        drop(prefs);
        // OIN-80: `leads[]` comes from the records — stalled and unloaded leads
        // appear, with state/health/idleReason/lastActiveAt (the UI LeadSchema).
        let live_session_ids: std::collections::HashSet<String> = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        let record_leads =
            crate::superagent_leads::lead_status_rows(self, &live_session_ids).await?;
        // OIN-81: the leads' open questions as in_progress needsYou rows. A
        // blocked-with-claim row (the timeout conversion) was already pushed in
        // the board pass above; drop an open-question row whose ticket has one.
        let timeout_minutes = crate::superagent_leads::LeadSettings::sanitized(config.lead.clone())
            .question_timeout_minutes;
        let open_questions =
            crate::superagent_questions::open_question_rows(self, &record_leads, timeout_minutes)
                .await;
        let mut needs_you = needs_you;
        for row in open_questions {
            if !needs_you
                .iter()
                .any(|existing| existing["taskId"] == row["taskId"])
            {
                needs_you.push(row);
            }
        }
        // Approval rows carry kind:"approval" (the UI's NeedsSchema).
        for row in &mut needs_you {
            if row.get("kind").is_none() {
                row["kind"] = json!("approval");
            }
        }
        // OIN-82: one board_created notice per project registered in the last
        // 24 h, computed from the project files (no new storage).
        let notices = self.board_created_notices();
        let lead_settings = config.lead.clone();
        // Record rows replace the claim-derived ones (one lead per project).
        Ok(
            json!({"enabled":config.enabled,"pausedGraphPolicy":config.paused_graph_policy,"workers":workers,"leads":record_leads,"needsYou":needs_you,"review":review,"projects":projects,"notices":notices,"lead":serde_json::to_value(crate::superagent_leads::LeadSettings::sanitized(lead_settings)).unwrap_or_default()}),
        )
    }

    /// OIN-82: `{kind:"board_created", projectId, name, at, text}` for projects
    /// whose `createdAt` is inside the last 24 hours (`at` as epoch-ms).
    fn board_created_notices(&self) -> Vec<Value> {
        let now = chrono::Utc::now();
        self.layout
            .list_projects()
            .unwrap_or_default()
            .into_iter()
            .filter(|project| !project.archived)
            .filter(|project| now.signed_duration_since(project.created_at).num_hours() < 24)
            .map(|project| {
                json!({
                    "kind": "board_created",
                    "projectId": project.slug,
                    "name": project.name,
                    "at": project.created_at.timestamp_millis(),
                    "text": format!("Board created for {} ({}).", project.name, project.slug),
                })
            })
            .collect()
    }
}

static DISPATCHER: OnceLock<Arc<Dispatcher>> = OnceLock::new();
/// # Errors
/// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
pub fn current() -> Result<&'static Arc<Dispatcher>> {
    DISPATCHER.get().ok_or_else(|| {
        fail(
            "E_DISPATCHER_UNAVAILABLE",
            "This host does not own dispatch; connect to the dispatcher host",
        )
    })
}
/// Q11: archive retire hook — a no-op when this host does not own dispatch;
/// otherwise disposes and retires every recorded lead of the archived chats.
pub async fn on_sessions_archived(ids: &[String]) {
    let Ok(dispatcher) = current() else { return };
    dispatcher.sessions_archived(ids).await;
}
/// Install before serving clients. A second host can serve other operations but not dispatch.
/// # Errors
/// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
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
/// # Errors
/// Returns an error for invalid dispatcher state or configuration, unavailable projects, or board and session operation failures.
#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
pub async fn request(req: unipi_proto::Request) -> Result<Value> {
    use unipi_proto::Request;
    let dispatcher = current()?;
    match req {
        Request::SuperagentStatus => dispatcher.status().await,
        Request::SuperagentConfigure {
            enabled,
            concurrency_cap,
            paused_graph_policy,
            lead,
        } => {
            let policy = match paused_graph_policy {
                Some(policy) => serde_json::from_value::<PausedGraphPolicy>(json!(policy))
                    .map_err(|_| fail("E_SCHEMA", "pausedGraphPolicy must be report or reuse"))?,
                None => dispatcher.paused_graph_policy().await,
            };
            let current = dispatcher.transaction.lock().await.clone();
            let lead_settings = match lead {
                Some(value) => {
                    let settings: crate::superagent_leads::LeadSettings =
                        serde_json::from_value(value).map_err(|e| fail("E_SCHEMA", e))?;
                    settings
                }
                None => current.lead.clone(),
            };
            serde_json::to_value(
                dispatcher
                    .configure(Config {
                        enabled,
                        concurrency_cap: concurrency_cap.unwrap_or_default(),
                        paused_graph_policy: policy,
                        lead: lead_settings,
                    })
                    .await?,
            )
            .map_err(storage)
        }
        Request::SuperagentConfigureProject {
            project_id,
            auto_start,
        } => dispatcher.configure_project(&project_id, auto_start).await,
        Request::SuperagentEnable {
            action,
            project_id,
            path,
        } => {
            dispatcher
                .enable(&action, project_id.as_deref(), path.as_deref())
                .await
        }
        Request::SuperagentConfigureProjectLead {
            project_id,
            model,
            thinking,
        } => {
            dispatcher
                .configure_project_lead(&project_id, model, thinking)
                .await
        }
        Request::SuperagentSetMode { project_id, mode } => {
            dispatcher.set_mode(&project_id, &mode).await
        }
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
            lead,
        } => {
            let tasks = if let Some(task) = task_id {
                if !task_ids.is_empty() {
                    return Err(fail("E_SCHEMA", "Use taskId or taskIds, not both"));
                }
                vec![task]
            } else {
                task_ids
            };
            if tasks.is_empty() {
                return Err(fail("E_SCHEMA", "taskIds must not be empty"));
            }
            let overrides = overrides
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| fail("E_SCHEMA", e))?
                .unwrap_or_default();
            if let Some(lead) = lead
                .as_deref()
                .filter(|lead| !lead.is_empty() && *lead != "auto")
            {
                if !matches!(lead, "reuse" | "new") {
                    return Err(fail("E_SCHEMA", "lead must be auto, reuse or new"));
                }
            }
            Ok(
                json!({"sessionId":dispatcher.start_tasks_with_lead(&project_id, &tasks, preset_id.as_deref(), overrides, lead.as_deref()).await?}),
            )
        }
        Request::SuperagentLeadAction {
            session_id,
            action,
            model,
        } => {
            crate::superagent_leads::lead_action(dispatcher, &session_id, &action, model.as_deref())
                .await
        }
        Request::SuperagentStop {
            project_id,
            task_id,
        } => Ok(json!({"stopped":dispatcher.stop(&project_id, &task_id).await?})),
        Request::SuperagentRelay {
            project_id,
            task_id,
            text,
            mut task_ids,
            delivery,
            priority,
            session_id,
            request_id,
        } => {
            if let Some(task) = task_id {
                if !task_ids.is_empty() {
                    return Err(fail("E_SCHEMA", "Use taskId or taskIds, not both"));
                }
                task_ids.push(task);
            }
            dispatcher
                .relay_to_lead_checked(
                    &project_id,
                    &text,
                    &task_ids,
                    Delivery::parse(delivery.as_deref().unwrap_or("queue"))?,
                    priority,
                    session_id.as_deref(),
                    request_id.as_deref(),
                )
                .await
        }
        Request::SuperagentAnswer {
            session_id,
            task_id,
            answer,
        } => {
            dispatcher
                .answer_blocked_question(&session_id, &task_id, &answer)
                .await
        }
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
