//! `write`.

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;
use crate::edit::storage::{TextFile, atomic_write, observe};
use crate::info::write_info;
use crate::path_utils::resolve_to_cwd;
use crate::{aborted, args, node_path};

/// The `write` tool.
#[derive(Debug)]
pub struct WriteTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl WriteTool {
    /// A `write` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: write_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for WriteTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: oino_ai::JsonObject) -> oino_ai::JsonObject {
        crate::legacy::normalize_call("write", &raw, &self.config.effective_cwd())
            .map_or(raw, |call| call.args)
    }

    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        async move { run(&config, &call, &cancel).await }.boxed()
    }
}

async fn run(
    config: &ToolConfig,
    call: &ToolInvocation,
    cancel: &CancellationToken,
) -> Result<AgentToolResult, ToolError> {
    let path = crate::legacy::file_path(&call.args)?;
    let content = args::required_str(&call.args, "content")?;
    let absolute = resolve_to_cwd(path, &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    let dir = node_path::dirname(&absolute);
    let env = config.env.as_ref();
    // Abort is checked after each step, never mid-operation, so the queue stays locked until
    // the file system call settled.
    let check = || {
        if cancel.is_cancelled() {
            Err(aborted())
        } else {
            Ok(())
        }
    };
    config
        .mutation_queue
        .run(env, &absolute, async {
            check()?;
            env.mkdir_p(&dir)
                .await
                .map_err(|error| ToolError::new(error.to_string()))?;
            check()?;
            if env.stat(&absolute).await.is_ok_and(|stat| stat.is_dir) {
                return Err(ToolError::new(format!(
                    "Cannot create file at \"{path}\" because a directory already exists there."
                )));
            }
            let previous = match env.read_file(&absolute).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.code == "ENOENT" => None,
                Err(error) => return Err(ToolError::new(error.to_string())),
            };
            let before = diagnostics(config, &absolute);
            let text = TextFile::decode(previous.as_deref().unwrap_or(content.as_bytes()))?;
            let bytes = text.encode(content.trim_start_matches('\u{feff}'), true);
            atomic_write(config, &absolute, &bytes, previous.as_deref()).await?;
            observe(config, &absolute, &bytes)?;
            check()?;
            let diagnostics = new_diagnostics(config, &absolute, &before);
            let result = serde_json::json!({"success": true, "file_path": absolute});
            Ok(AgentToolResult {
                structured_content: Some(result.clone()),
                details: Some(serde_json::json!({"new_diagnostics": diagnostics})),
                ..AgentToolResult::text(format!("{result}{}", diagnostics_reminder(&diagnostics)))
            })
        })
        .await
}

/// Host hook for diagnostics snapshots. The host may use the config's session context
/// to route diagnostics to the appropriate language server. Unchanged diagnostics are
/// never repeated in mutation results.
pub trait DiagnosticsHook: Send + Sync {
    /// Current diagnostics for this file, as stable display strings.
    fn diagnostics(&self, config: &ToolConfig, path: &str) -> Vec<String>;
}

static DIAGNOSTICS: std::sync::RwLock<Option<std::sync::Arc<dyn DiagnosticsHook>>> =
    std::sync::RwLock::new(None);

/// Install (or remove) the host's optional diagnostics hook.
///
/// # Errors
/// Returns an error if a previous hook installer poisoned the lock.
pub fn set_diagnostics_hook(
    hook: Option<std::sync::Arc<dyn DiagnosticsHook>>,
) -> Result<(), ToolError> {
    *DIAGNOSTICS
        .write()
        .map_err(|_| ToolError::new("Diagnostics hook lock poisoned"))? = hook;
    Ok(())
}

pub(crate) fn diagnostics(config: &ToolConfig, path: &str) -> Vec<String> {
    let hook = DIAGNOSTICS.read().ok().and_then(|hook| hook.clone());
    hook.map_or_else(Vec::new, |hook| hook.diagnostics(config, path))
}

pub(crate) fn new_diagnostics(config: &ToolConfig, path: &str, before: &[String]) -> Vec<String> {
    diagnostics(config, path)
        .into_iter()
        .filter(|diagnostic| !before.contains(diagnostic))
        .collect()
}

pub(crate) fn diagnostics_reminder(diagnostics: &[String]) -> String {
    if diagnostics.is_empty() {
        String::new()
    } else {
        format!("\nNew diagnostics:\n{}", diagnostics.join("\n"))
    }
}
