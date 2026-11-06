//! Built-in tools of a session (`oino-tools` wired to the session's settings and context).

use std::sync::{Arc, OnceLock, Weak};

use oino_tools::{
    BashDetachRegistry, ImageOptions, ModelImageInfo, SessionEnv, ShellOptions, ToolConfig,
    ToolContext, builtin_tool_infos, builtin_tools,
};

use crate::session::{AgentSession, Inner, ToolEntry};

/// Session context the built-in tools read at call time (cwd, model image limits, D12 env).
#[derive(Default)]
pub struct SessionToolContext {
    session: OnceLock<Weak<Inner>>,
    /// Detach registry shared by the session's shell tools (`control.bash-detach`).
    bash_detach: BashDetachRegistry,
}

impl std::fmt::Debug for SessionToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionToolContext").finish_non_exhaustive()
    }
}

impl SessionToolContext {
    /// Binds the context to its session (once).
    pub fn bind(&self, session: &AgentSession) {
        let _ = self.session.set(Arc::downgrade(&session.inner));
        session.bind_bash_detach(self.bash_detach.clone());
    }

    fn session(&self) -> Option<AgentSession> {
        self.session
            .get()
            .and_then(Weak::upgrade)
            .map(AgentSession::from_inner)
    }
}

impl ToolContext for SessionToolContext {
    fn codemode_catalog(&self) -> Vec<oino_ai::JsonValue> {
        self.session()
            .map_or_else(Vec::new, |s| s.codemode_catalog())
    }

    fn codemode_models(&self, available_only: bool) -> Vec<oino_ai::JsonValue> {
        self.session().map_or_else(Vec::new, |session| {
            let models = if available_only {
                session.models().available()
            } else {
                session.models().all()
            };
            models
                .into_iter()
                .filter_map(|model| serde_json::to_value(model).ok())
                .collect()
        })
    }

    fn execute_nested(
        &self,
        parent: String,
        name: String,
        args: oino_ai::JsonObject,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<
        'static,
        Result<oino_agent::AgentToolResult, oino_agent::ToolError>,
    > {
        let session = self.session();
        Box::pin(async move {
            let session =
                session.ok_or_else(|| oino_agent::ToolError::new("No session tool service"))?;
            let outcome = session
                .execute_nested_tool(&parent, &name, args, cancel)
                .await;
            if outcome.is_error {
                return Err(oino_agent::ToolError::new(
                    outcome
                        .result
                        .content
                        .iter()
                        .filter_map(|v| match v {
                            oino_ai::UserContent::Text(t) => Some(t.text.as_str()),
                            oino_ai::UserContent::Image(_) => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
            }
            Ok(outcome.result)
        })
    }

    fn search_tools(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<oino_ai::JsonValue, oino_agent::ToolError> {
        self.session()
            .ok_or_else(|| oino_agent::ToolError::new("No session tool service"))?
            .search_deferred_tools(query, limit)
    }

    fn codemode_store(&self) -> oino_ai::JsonObject {
        self.session()
            .map_or_else(oino_ai::JsonObject::new, |session| {
                session.with_manager(|manager| {
                    manager
                        .branch(None)
                        .iter()
                        .rev()
                        .find_map(|entry| match &entry.kind {
                            oino_session::EntryKind::Custom {
                                custom_type,
                                data: Some(data),
                            } if custom_type == "codemode-store" => data.as_object().cloned(),
                            _ => None,
                        })
                        .unwrap_or_default()
                })
            })
    }

    fn commit_codemode_store(
        &self,
        values: oino_ai::JsonObject,
    ) -> Result<(), oino_agent::ToolError> {
        let session = self
            .session()
            .ok_or_else(|| oino_agent::ToolError::new("No session store service"))?;
        session
            .with_manager(|manager| {
                manager.append_custom_entry(
                    "codemode-store",
                    Some(oino_ai::JsonValue::Object(values)),
                )?;
                manager.sync()
            })
            .map_err(|e| oino_agent::ToolError::new(e.to_string()))
    }

    fn background_shell(
        &self,
        args: oino_ai::JsonValue,
    ) -> futures::future::BoxFuture<
        'static,
        Result<oino_agent::AgentToolResult, oino_agent::ToolError>,
    > {
        let session = self.session();
        Box::pin(async move {
            let session =
                session.ok_or_else(|| oino_agent::ToolError::new("No session task service"))?;
            let value = session
                .service_call(Some("unipi.background-tasks"), "tasks.shell", args)
                .await
                .map_err(|e| oino_agent::ToolError::new(e.to_string()))?;
            serde_json::from_value(value).map_err(|e| oino_agent::ToolError::new(e.to_string()))
        })
    }

    fn cwd(&self) -> Option<String> {
        self.session().map(|s| s.work_cwd().display().to_string())
    }

    fn model_images(&self) -> Option<ModelImageInfo> {
        let session = self.session()?;
        let model = session.routed_model().or_else(|| session.model())?;
        Some(ModelImageInfo::from(&model))
    }

    fn session_env(&self) -> Option<SessionEnv> {
        let session = self.session()?;
        let model = session.model();
        Some(SessionEnv {
            session_id: session.session_id(),
            session_file: session.session_file().map(|p| p.display().to_string()),
            provider: model.as_ref().map(|m| m.provider.clone()),
            model: model.as_ref().map(|m| m.id.clone()),
            reasoning_level: Some(session.thinking_level().to_string()),
        })
    }
}

/// Every built-in tool as session tool entries.
#[must_use]
pub fn builtin_tool_entries(
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
    settings: &oino_settings::Settings,
    context: Arc<SessionToolContext>,
) -> Vec<ToolEntry> {
    builtin_tool_entries_with_search(
        cwd,
        agent_dir,
        settings,
        context,
        settings.semantic_search_enabled(),
    )
}

/// Construct built-ins from the persisted semantic-search session snapshot.
#[must_use]
pub fn builtin_tool_entries_with_search(
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
    settings: &oino_settings::Settings,
    context: Arc<SessionToolContext>,
    semantic_enabled: bool,
) -> Vec<ToolEntry> {
    let mut config = ToolConfig::new(cwd.display().to_string(), agent_dir.display().to_string());
    config.search.enabled = semantic_enabled;
    config.bash_detach = context.bash_detach.clone();
    config.context = context;
    config.images = ImageOptions {
        auto_resize: settings.image_settings().auto_resize,
        fallback_resize: None,
    };
    config.shell = ShellOptions {
        shell_path: settings.shell_path().map(|p| p.display().to_string()),
        command_prefix: settings.shell_command_prefix().map(str::to_owned),
        expose_session_environment: true,
    };
    let infos = builtin_tool_infos(&config);
    builtin_tools(&config)
        .into_iter()
        .zip(infos)
        .map(|(tool, info)| {
            ToolEntry::builtin(
                tool,
                info.prompt_snippet.map(str::to_owned),
                info.prompt_guidelines.unwrap_or_default(),
            )
        })
        .collect()
}

/// Settings-only entry point. Not registered in any model or session command catalog.
/// # Errors
/// Returns consent, engine availability, and indexing errors.
pub async fn build_search_index(
    root: &std::path::Path,
    agent_dir: &std::path::Path,
    consent: bool,
) -> Result<(), String> {
    oino_tools::zvec::build_index(root, agent_dir, consent).await
}

/// Freeze the feature for reloads. Legacy sessions without a snapshot remain exact.
/// # Errors
/// Returns session persistence errors.
pub fn search_snapshot(
    manager: &mut oino_session::SessionManager,
    enabled: bool,
) -> Result<bool, oino_session::SessionError> {
    for entry in manager.entries() {
        if let oino_session::EntryKind::Custom { custom_type, data } = &entry.kind
            && custom_type == "oino.search-snapshot"
        {
            return Ok(data
                .as_ref()
                .and_then(|v| v["semanticEnabled"].as_bool())
                .unwrap_or(false));
        }
    }
    let enabled = enabled && manager.build_session_context().messages.is_empty();
    manager.append_custom_entry(
        "oino.search-snapshot",
        Some(serde_json::json!({"semanticEnabled": enabled})),
    )?;
    Ok(enabled)
}
