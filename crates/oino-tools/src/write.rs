//! `write` (pi `tools/write.ts`).

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;
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
    let path = args::required_str(&call.args, "path")?;
    let content = args::required_str(&call.args, "content")?;
    let absolute = resolve_to_cwd(path, &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    let dir = node_path::dirname(&absolute);
    let env = config.env.as_ref();
    // Abort is checked after each step, never mid-operation, so the queue stays locked until
    // the file system call settled (pi).
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
            env.write_file(&absolute, content.as_bytes())
                .await
                .map_err(|error| ToolError::new(error.to_string()))?;
            check()?;
            Ok(AgentToolResult::text(format!(
                "Successfully wrote to {path}"
            )))
        })
        .await
}
