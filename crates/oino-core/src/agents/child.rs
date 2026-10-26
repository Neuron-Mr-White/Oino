//! Spawning and driving one nested agent session.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use oino_ai::{AgentMessage, JsonValue, Model, ThinkingLevel};
use oino_session::{CreateOptions, SessionManager};
use serde_json::json;
use tokio::task::JoinHandle;

use super::spec::{
    AgentSpec, CLOSE_GRACE_CAP, DEFAULT_MAX_CONCURRENT, DEFAULT_MAX_DEPTH, ExtensionPolicy,
    HARD_MAX_CONCURRENT, HARD_MAX_DEPTH, Persistence, Seed,
};
use super::{ChildConfig, ChildLink, Tree};
use crate::builtin::{SessionToolContext, builtin_tool_entries};
use crate::events::{AgentSessionEvent, ClientEvent};
use crate::extensions::api::CallContext;
use crate::extensions::host::HostError;
use crate::extensions::runner::ExtensionSet;
use crate::session::{
    AgentSession, Inner, InputSource, PromptDisposition, PromptOptions, SessionConfig,
    StreamingBehavior, lock,
};

fn failed(error: impl std::fmt::Display) -> HostError {
    HostError::Failed(error.to_string())
}

/// A nested agent session owned by an extension (`agents.agent`).
pub struct ChildAgent {
    id: String,
    label: String,
    owner: String,
    session: AgentSession,
    parent: Weak<Inner>,
    tree: Arc<Tree>,
    closed: AtomicBool,
    forward: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for ChildAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildAgent")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl ChildAgent {
    /// Agent id (the child's session id).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Label from the spec.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Extension that spawned the agent.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The child session (transcript, entries, stats).
    #[must_use]
    pub fn session(&self) -> &AgentSession {
        &self.session
    }

    /// The child's transcript.
    #[must_use]
    pub fn messages(&self) -> Vec<AgentMessage> {
        self.session.messages()
    }

    /// Child session file (`None` in memory).
    #[must_use]
    pub fn session_file(&self) -> Option<PathBuf> {
        self.session.session_file()
    }

    /// Whether [`ChildAgent::close`] ran.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// `running`, `idle` or `closed`.
    #[must_use]
    pub fn state(&self) -> &'static str {
        if self.is_closed() {
            "closed"
        } else if self.session.is_idle() {
            "idle"
        } else {
            "running"
        }
    }

    fn ensure_open(&self) -> Result<(), HostError> {
        if self.is_closed() {
            Err(HostError::Failed(format!("Agent {} is closed", self.id)))
        } else {
            Ok(())
        }
    }

    /// Prompts the child; while it runs the text is queued (`steer`, else follow-up).
    /// Returns the disposition (`started`, `queued`, `handled`).
    ///
    /// # Errors
    ///
    /// Closed agent and pi prompt errors (no model, no API key, …).
    pub async fn prompt(&self, text: String, delivery: Option<&str>) -> Result<String, HostError> {
        self.ensure_open()?;
        let behavior = if delivery == Some("steer") {
            StreamingBehavior::Steer
        } else {
            StreamingBehavior::FollowUp
        };
        let disposition = self
            .session
            .prompt(
                text,
                PromptOptions {
                    streaming_behavior: Some(behavior),
                    source: InputSource::Extension,
                    ..PromptOptions::default()
                },
            )
            .await
            .map_err(failed)?;
        Ok(match disposition {
            PromptDisposition::Handled => "handled",
            PromptDisposition::Queued => "queued",
            PromptDisposition::Started => "started",
        }
        .to_owned())
    }

    /// Steering message while running; starts a run when idle.
    ///
    /// # Errors
    ///
    /// As [`ChildAgent::prompt`].
    pub async fn steer(&self, text: String) -> Result<(), HostError> {
        self.prompt(text, Some("steer")).await.map(|_| ())
    }

    /// Follow-up message while running; starts a run when idle (resume).
    ///
    /// # Errors
    ///
    /// As [`ChildAgent::prompt`].
    pub async fn follow_up(&self, text: String) -> Result<(), HostError> {
        self.prompt(text, Some("followUp")).await.map(|_| ())
    }

    /// Aborts the current run (returns at once).
    pub fn abort(&self) {
        let session = self.session.clone();
        tokio::spawn(async move { session.abort().await });
    }

    /// Sets the thinking level; returns the level clamped to the child's model.
    ///
    /// # Errors
    ///
    /// Unknown level.
    pub fn set_thinking(&self, level: &str) -> Result<String, HostError> {
        let level: ThinkingLevel = level
            .parse()
            .map_err(|_| HostError::Invalid(format!("thinking level `{level}`")))?;
        let clamped = level.clamp(&self.session.available_thinking_levels());
        let session = self.session.clone();
        tokio::spawn(async move {
            session.set_thinking_level(clamped, false).await;
        });
        Ok(clamped.to_string())
    }

    /// Text of the child's last assistant message.
    #[must_use]
    pub fn last_assistant_text(&self) -> Option<String> {
        self.session.last_assistant_text()
    }

    /// Resolves once the child is idle (immediately when closed).
    pub async fn wait_idle(&self) {
        if !self.is_closed() {
            self.session.wait_for_idle().await;
        }
    }

    /// The child's `SessionStats` JSON.
    #[must_use]
    pub fn usage(&self) -> JsonValue {
        serde_json::to_value(self.session.session_stats()).unwrap_or(JsonValue::Null)
    }

    /// Child client events (pi `AgentSessionEvent` JSON and friends) from now until close.
    pub fn events(&self) -> BoxStream<'static, JsonValue> {
        if self.is_closed() {
            return Box::pin(futures::stream::empty());
        }
        Box::pin(
            self.session
                .subscribe()
                .map(|event| serde_json::to_value(&event).unwrap_or(JsonValue::Null)),
        )
    }

    /// Closes the agent: waits up to `grace` (≤ 5 s) for idle, aborts, emits
    /// `session_shutdown{reason:"quit"}` to the child's extensions, disposes the session and
    /// ends its event streams. Idempotent.
    pub async fn close(&self, grace: Duration) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let grace = grace.min(CLOSE_GRACE_CAP);
        if !self.session.is_idle()
            && tokio::time::timeout(grace, self.session.wait_for_idle())
                .await
                .is_err()
        {
            self.session.abort().await;
        }
        let runner = self.session.runner();
        if runner.has_handlers("session_shutdown") {
            runner
                .emit(json!({"type": "session_shutdown", "reason": "quit"}))
                .await;
        }
        if let Some(router) = self.session.inner.agents.router() {
            router.detach(&self.id);
        }
        self.session.dispose().await;
        self.session.hub().close();
        if let Some(task) = lock(&self.forward).take() {
            task.abort();
        }
        if let Some(parent) = self.parent.upgrade() {
            parent.agents.remove_child(&self.id);
        }
        self.tree.release();
    }
}

/// `extensionSettings.subagents.maxConcurrent` (default 8, hard cap 16).
fn max_concurrent(session: &AgentSession) -> usize {
    session
        .with_settings(|s| {
            s.settings()
                .merged()
                .get("extensionSettings")
                .and_then(|e| e.get("subagents"))
                .and_then(|e| e.get("maxConcurrent"))
                .and_then(JsonValue::as_u64)
        })
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT)
        .clamp(1, HARD_MAX_CONCURRENT)
}

fn context_messages(session: &AgentSession) -> Vec<AgentMessage> {
    session
        .with_manager(|m| m.build_session_context().messages)
        .into_iter()
        .filter(|m| !matches!(m, AgentMessage::System(_)))
        .collect()
}

impl AgentSession {
    fn resolve_child_model(&self, spec: &AgentSpec) -> Result<Model, HostError> {
        match &spec.model {
            Some(key) => key
                .split_once('/')
                .and_then(|(provider, id)| self.inner.models.find(provider, id))
                .ok_or_else(|| HostError::Invalid(format!("Unknown model {key}"))),
            None => self
                .model()
                .ok_or_else(|| HostError::Failed("No model selected".into())),
        }
    }

    fn child_manager(&self, spec: &AgentSpec, cwd: &Path) -> Result<SessionManager, HostError> {
        let parent_file = self.session_file();
        let options = CreateOptions {
            id: None,
            parent_session: parent_file.clone(),
        };
        let dir = match &spec.persistence {
            Persistence::Memory => None,
            Persistence::File { session_dir } => session_dir.clone().or_else(|| {
                parent_file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(|d| d.join("children"))
            }),
        };
        let seed_messages = |manager: &mut SessionManager, messages: Vec<AgentMessage>| {
            for message in messages {
                manager.append_message(message).map_err(failed)?;
            }
            Ok::<(), HostError>(())
        };
        let manager = match (&spec.seed, dir) {
            (Seed::SessionFile(path), Some(dir)) => {
                SessionManager::open(path, &dir, Some(cwd)).map_err(failed)?
            }
            (Seed::SessionFile(path), None) => {
                let root = path.parent().unwrap_or(cwd);
                let source = SessionManager::open(path, root, Some(cwd)).map_err(failed)?;
                SessionManager::in_memory(cwd, options, source.entries().to_vec())
            }
            (Seed::BranchCopy, None) => {
                let branch = self.with_manager(|m| {
                    m.branch(None)
                        .into_iter()
                        .filter(|e| !matches!(e.message(), Some(AgentMessage::System(_))))
                        .cloned()
                        .collect()
                });
                SessionManager::in_memory(cwd, options, branch)
            }
            (seed, dir) => {
                let mut manager = match dir {
                    Some(dir) => SessionManager::create(cwd, &dir, options).map_err(failed)?,
                    None => SessionManager::in_memory(cwd, options, Vec::new()),
                };
                match seed {
                    Seed::BranchCopy => seed_messages(&mut manager, context_messages(self))?,
                    Seed::Messages(messages) => seed_messages(&mut manager, messages.clone())?,
                    Seed::None | Seed::SessionFile(_) => {}
                }
                manager
            }
        };
        Ok(manager)
    }

    fn child_extensions(&self, policy: &ExtensionPolicy) -> Arc<ExtensionSet> {
        let all = &self.inner.agents.all_extensions;
        match policy {
            ExtensionPolicy::None => Arc::new(ExtensionSet::default()),
            ExtensionPolicy::Inherit => Arc::clone(all),
            ExtensionPolicy::Only(ids) => Arc::new(ExtensionSet::new(
                all.extensions()
                    .iter()
                    .filter(|e| ids.iter().any(|id| id == e.instance.id()))
                    .cloned()
                    .collect(),
            )),
        }
    }

    /// `agents.spawn` (runtime §12): creates a nested session owned by `extension`.
    ///
    /// # Errors
    ///
    /// Depth or concurrency limit, unknown model, session file failures.
    #[allow(clippy::too_many_lines, reason = "one step per AgentSpec field")]
    pub async fn spawn_agent(
        &self,
        extension: &str,
        spec: AgentSpec,
    ) -> Result<Arc<ChildAgent>, HostError> {
        let agents = &self.inner.agents;
        let depth = agents.depth.saturating_add(1);
        if depth > agents.max_depth {
            return Err(HostError::Failed(format!(
                "Nested agent depth limit reached ({} levels)",
                agents.max_depth
            )));
        }
        let model = self.resolve_child_model(&spec)?;
        let thinking = spec
            .thinking
            .unwrap_or_else(|| self.thinking_level())
            .clamp(&model.available_thinking_levels());
        let cwd = spec.cwd.clone().unwrap_or_else(|| self.cwd().to_path_buf());
        let max = max_concurrent(self);
        if !agents.tree.try_acquire(max) {
            return Err(HostError::Failed(format!(
                "Too many nested agents running (max {max})"
            )));
        }
        let built = self.build_child(extension, &spec, model, thinking, &cwd, depth);
        let child = match built {
            Ok(child) => child,
            Err(error) => {
                agents.tree.release();
                return Err(error);
            }
        };
        if let Some(router) = agents.router() {
            router.attach(&child);
        }
        child.bind_extensions("startup", None).await;
        let forward = tokio::spawn(forward_child_events(
            child.subscribe(),
            Arc::downgrade(&self.inner),
            child.inner.agents.link.clone(),
            extension.to_owned(),
            child.session_id(),
            self.runner().call_context().clone(),
        ));
        let handle = Arc::new(ChildAgent {
            id: child.session_id(),
            label: spec.label.clone(),
            owner: extension.to_owned(),
            session: child,
            parent: Arc::downgrade(&self.inner),
            tree: Arc::clone(&agents.tree),
            closed: AtomicBool::new(false),
            forward: Mutex::new(Some(forward)),
        });
        lock(&agents.children).push(Arc::clone(&handle));
        Ok(handle)
    }

    fn build_child(
        &self,
        extension: &str,
        spec: &AgentSpec,
        model: Model,
        thinking: ThinkingLevel,
        cwd: &Path,
        depth: u8,
    ) -> Result<AgentSession, HostError> {
        let agents = &self.inner.agents;
        let mut manager = self.child_manager(spec, cwd)?;
        manager
            .append_model_change(&model.provider, &model.id)
            .map_err(failed)?;
        manager
            .append_thinking_level_change(&thinking.to_string())
            .map_err(failed)?;
        let settings = self.with_settings(|s| s.snapshot());
        let wanted: Vec<String> = spec.tools.builtin.clone().unwrap_or_else(|| {
            settings.default_tools().unwrap_or_else(|| {
                oino_settings::tools::DEFAULT_TOOL_NAMES
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            })
        });
        let context = Arc::new(SessionToolContext::default());
        let mut builtin =
            builtin_tool_entries(cwd, self.agent_dir(), &settings, Arc::clone(&context));
        builtin.retain(|entry| wanted.iter().any(|w| w == entry.name()));
        let mut allowed = wanted.clone();
        allowed.extend(spec.tools.extension_tools.iter().cloned());
        let parent_cx = self.runner().call_context().clone();
        let has_ui = match spec.ui {
            super::UiPolicy::Forward => parent_cx.has_ui,
            super::UiPolicy::AutoDeny { .. } => true,
            super::UiPolicy::None => false,
        };
        let child_role = spec
            .child_role
            .clone()
            .or_else(|| (!extension.is_empty()).then(|| extension.to_owned()));
        let max_depth = spec
            .max_depth
            .unwrap_or(DEFAULT_MAX_DEPTH)
            .min(agents.max_depth)
            .min(HARD_MAX_DEPTH);
        let session = AgentSession::new(SessionConfig {
            cwd: cwd.to_path_buf(),
            agent_dir: self.agent_dir().to_path_buf(),
            docs_root: self.inner.docs_root.clone(),
            manager,
            settings: Arc::clone(&self.inner.settings),
            resources: Arc::clone(&self.inner.resources),
            models: self.inner.models.clone(),
            extensions: self.child_extensions(&spec.extensions),
            stream: Arc::clone(&agents.base_stream),
            mode: self.mode(),
            has_ui,
            builtin_tools: builtin,
            custom_tools: Vec::new(),
            initial_active_tools: wanted,
            allowed_tools: Some(allowed),
            excluded_tools: spec.tools.deny.clone(),
            model: Some(model),
            thinking_level: thinking,
            scoped_models: Vec::new(),
            flag_values: self.runner().flag_values(),
            child: Some(ChildConfig {
                link: ChildLink {
                    parent: Arc::downgrade(&self.inner),
                    label: spec.label.clone(),
                    role: child_role,
                    ui: spec.ui.clone(),
                    prompt: spec.system_prompt.clone(),
                },
                all_extensions: Arc::clone(&agents.all_extensions),
                base_stream: Arc::clone(&agents.base_stream),
                tree: Arc::clone(&agents.tree),
                depth,
                max_depth,
                router: agents.router.get().cloned(),
            }),
        });
        context.bind(&session);
        Ok(session)
    }

    /// Nested agents of this session (`agents.list`): `[{id, label, state, sessionFile}]`.
    #[must_use]
    pub fn agents_list(&self) -> JsonValue {
        let children = lock(&self.inner.agents.children).clone();
        JsonValue::Array(
            children
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id(),
                        "label": c.label(),
                        "state": c.state(),
                        "owner": c.owner(),
                        "sessionFile": c.session_file().map(|p| p.display().to_string()),
                    })
                })
                .collect(),
        )
    }

    /// Open nested agents of this session.
    #[must_use]
    pub fn child_agents(&self) -> Vec<Arc<ChildAgent>> {
        lock(&self.inner.agents.children).clone()
    }

    /// Closes every nested agent (session disposal).
    pub(crate) async fn close_children(&self) {
        let children = std::mem::take(&mut *lock(&self.inner.agents.children));
        for child in children {
            child.close(Duration::ZERO).await;
        }
    }

    /// Whether this is a nested (child) session.
    #[must_use]
    pub fn is_child(&self) -> bool {
        self.inner.agents.link.is_some()
    }
}

/// Mirrors child usage into the parent (`usage{kind:"child:<label>"}`) and delivers
/// `oino_agent` events to the owner when it subscribed.
async fn forward_child_events(
    mut events: crate::hub::EventStream,
    parent: Weak<Inner>,
    link: Option<ChildLink>,
    owner: String,
    agent_id: String,
    owner_cx: CallContext,
) {
    let label = link.map(|l| l.label).unwrap_or_default();
    let kind = format!("child:{label}");
    while let Some(event) = events.next().await {
        let Some(parent) = parent.upgrade().map(AgentSession::from_inner) else {
            return;
        };
        if let ClientEvent::Session(AgentSessionEvent::MessageEnd {
            message: AgentMessage::Assistant(message),
        }) = &event
        {
            let appended = parent.with_manager(|m| {
                m.append_usage(
                    &kind,
                    &message.provider,
                    &message.model,
                    message.usage,
                    None,
                )
            });
            match appended {
                Ok(id) => parent.publish_entry(&id),
                Err(error) => tracing::warn!("recording child usage failed: {error}"),
            }
        }
        let owner_ext = parent
            .runner()
            .set()
            .get(&owner)
            .cloned()
            .or_else(|| parent.inner.agents.all_extensions.get(&owner).cloned());
        if let Some(ext) = owner_ext
            && ext.state().events.contains("oino_agent")
        {
            let payload = json!({"type": "oino_agent", "agentId": agent_id, "event": event});
            if let Err(error) = ext
                .instance
                .on_event("oino_agent", payload, owner_cx.clone())
                .await
            {
                tracing::debug!("oino_agent delivery failed: {}", error.message);
            }
        }
    }
}
