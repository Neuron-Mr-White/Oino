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
    fn cwd(&self) -> Option<String> {
        self.session().map(|s| s.cwd().display().to_string())
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

/// Every built-in tool (pi `createAllToolDefinitions`) as session tool entries.
#[must_use]
pub fn builtin_tool_entries(
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
    settings: &oino_settings::Settings,
    context: Arc<SessionToolContext>,
) -> Vec<ToolEntry> {
    let mut config = ToolConfig::new(cwd.display().to_string(), agent_dir.display().to_string());
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
