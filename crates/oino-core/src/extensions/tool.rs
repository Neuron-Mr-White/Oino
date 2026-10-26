//! Extension tools as [`AgentTool`]s (pi `wrapRegisteredTools`).

use std::sync::Arc;

use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ExecutionMode, ToolDefinitionMeta, ToolError, ToolInvocation,
    UpdateSink,
};
use oino_ai::JsonObject;
use tokio_util::sync::CancellationToken;

use super::api::{CallContext, CallKind, ToolSpec};
use super::runner::LoadedExtension;

/// An extension tool bound to a session's call context.
pub struct ExtensionTool {
    meta: ToolDefinitionMeta,
    spec: ToolSpec,
    extension: Arc<LoadedExtension>,
    cx: CallContext,
}

impl std::fmt::Debug for ExtensionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionTool")
            .field("name", &self.meta.name)
            .field("extension", &self.extension.instance.id())
            .finish_non_exhaustive()
    }
}

impl ExtensionTool {
    /// Wraps `spec` of `extension`.
    #[must_use]
    pub fn new(spec: ToolSpec, extension: Arc<LoadedExtension>, cx: &CallContext) -> Self {
        let mut meta = ToolDefinitionMeta::new(
            spec.name.clone(),
            if spec.label.is_empty() {
                spec.name.clone()
            } else {
                spec.label.clone()
            },
            spec.description.clone(),
            spec.parameters.clone(),
        );
        meta.output_schema.clone_from(&spec.output_schema);
        meta.execution_mode = spec.execution_mode.unwrap_or(ExecutionMode::Parallel);
        Self {
            meta,
            spec,
            extension,
            cx: cx.with_kind(CallKind::Tool),
        }
    }

    /// The registration.
    #[must_use]
    pub const fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    /// Owning extension id.
    #[must_use]
    pub fn extension_id(&self) -> &str {
        self.extension.instance.id()
    }
}

impl AgentTool for ExtensionTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: JsonObject) -> JsonObject {
        if self.spec.prepare_arguments {
            self.extension
                .instance
                .prepare_arguments(&self.spec.name, raw)
        } else {
            raw
        }
    }

    fn execute(
        &self,
        call: ToolInvocation,
        updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let extension = Arc::clone(&self.extension);
        let name = self.spec.name.clone();
        let cx = self.cx.clone();
        Box::pin(async move {
            extension
                .instance
                .run_tool(&name, call, updates, cancel, cx)
                .await
                .map_err(|error| ToolError::new(error.message))
        })
    }
}
