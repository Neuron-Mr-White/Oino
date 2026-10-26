//! `ls` (pi `tools/ls.ts`).

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::JsonValue;
use serde_json::{Map, json};
use tokio_util::sync::CancellationToken;

use crate::collate::locale_compare;
use crate::config::ToolConfig;
use crate::env::path_exists;
use crate::info::{LS_DEFAULT_LIMIT, ls_info};
use crate::js::number_to_string;
use crate::path_utils::resolve_to_cwd;
use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, format_size, truncate_head};
use crate::{aborted, args, js_number, node_path};

/// The `ls` tool.
#[derive(Debug)]
pub struct LsTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl LsTool {
    /// An `ls` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: ls_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for LsTool {
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
        async move {
            if cancel.is_cancelled() {
                return Err(aborted());
            }
            tokio::select! {
                result = run(&config, &call) => result,
                () = cancel.cancelled() => Err(aborted()),
            }
        }
        .boxed()
    }
}

async fn run(config: &ToolConfig, call: &ToolInvocation) -> Result<AgentToolResult, ToolError> {
    let path = args::optional_str(&call.args, "path").filter(|path| !path.is_empty());
    let limit = args::optional_f64(&call.args, "limit").unwrap_or(LS_DEFAULT_LIMIT);
    let env = config.env.as_ref();
    let dir = resolve_to_cwd(path.unwrap_or("."), &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    if !path_exists(env, &dir).await {
        return Err(ToolError::new(format!("Path not found: {dir}")));
    }
    let stat = env
        .stat(&dir)
        .await
        .map_err(|error| ToolError::new(error.to_string()))?;
    if !stat.is_dir {
        return Err(ToolError::new(format!("Not a directory: {dir}")));
    }
    let mut entries = env
        .read_dir(&dir)
        .await
        .map_err(|error| ToolError::new(format!("Cannot read directory: {error}")))?;
    entries.sort_by(|a, b| locale_compare(&a.to_lowercase(), &b.to_lowercase()));
    let mut results: Vec<String> = Vec::new();
    let mut limit_reached = false;
    for entry in entries {
        #[allow(
            clippy::cast_precision_loss,
            reason = "entry counts are far below 2^53"
        )]
        if results.len() as f64 >= limit {
            limit_reached = true;
            break;
        }
        let Ok(entry_stat) = env.stat(&node_path::join(&dir, &entry)).await else {
            continue;
        };
        results.push(if entry_stat.is_dir {
            format!("{entry}/")
        } else {
            entry
        });
    }
    if results.is_empty() {
        return Ok(AgentToolResult::text("(empty directory)"));
    }
    let truncation = truncate_head(&results.join("\n"), TruncationOptions::bytes_only());
    let mut output = truncation.content.clone();
    let mut details = Map::new();
    let mut notices = Vec::new();
    if limit_reached {
        notices.push(format!(
            "{} entries limit reached. Use limit={} for more",
            number_to_string(limit),
            number_to_string(limit * 2.0)
        ));
        details.insert("entryLimitReached".to_owned(), js_number(limit));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert("truncation".to_owned(), json!(truncation));
    }
    if !notices.is_empty() {
        output = format!("{output}\n\n[{}]", notices.join(". "));
    }
    Ok(AgentToolResult {
        details: (!details.is_empty()).then_some(JsonValue::Object(details)),
        ..AgentToolResult::text(output)
    })
}
