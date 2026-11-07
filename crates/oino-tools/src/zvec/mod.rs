//! Opt-in, in-process semantic search. Indexing is a user-interface operation, never a tool.

use std::path::{Path, PathBuf};

mod client;
mod install;
/// Install a pinned helper after explicit consent.
/// # Errors
/// Returns download, integrity, or installation errors.
pub async fn install_helper(
    agent_dir: &Path,
    consent: bool,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(), String> {
    install::install(agent_dir, consent, cancel).await
}
/// Remove the pinned helper without deleting models or indexes.
/// # Errors
/// Returns filesystem errors.
pub fn uninstall_helper(agent_dir: &Path) -> Result<(), String> {
    if let Some((version, _)) = client::asset() {
        let path = agent_dir.join("tools/zvec").join(version);
        if path.exists() {
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
/// Current installation byte progress.
#[must_use]
pub fn install_progress() -> (u64, u64) {
    install::progress()
}
/// Information about the pinned helper and its installation.
#[must_use]
pub fn capabilities(agent_dir: &Path) -> serde_json::Value {
    client::capabilities(agent_dir)
}
/// Actionable refusal when the helper is absent.
pub const INSTALL: &str =
    "Semantic search is not installed. Open Settings › Search to install semantic search.";

/// The only supported embedding model (local, CPU).
pub const MODEL: &str = "local/potion-code-16m-v2";
/// Actionable refusal shared by all search entry points.
pub const BUILD_INDEX: &str = "Semantic search needs an index and a downloaded model. Open Settings › Search › 'Build index' and consent to build it.";
/// Models are released five minutes after the last operation completes.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Frozen session option. Changing settings never changes an existing instance.
#[derive(Clone, Debug, Default)]
pub struct SearchConfig {
    /// Snapshot of search.semanticEnabled.
    pub enabled: bool,
}

/// Metadata is conditional on the session snapshot, not live settings.
#[must_use]
pub fn grep_info(enabled: bool) -> crate::BuiltinToolInfo {
    let mut info = crate::info::grep_info();
    if enabled {
        info.parameters["properties"]["search_mode"] = serde_json::json!({
            "type": "string", "enum": ["exact", "semantic", "hybrid"], "default": "exact"
        });
        info.description.push_str(" search_mode defaults to exact. Semantic/hybrid requires a user-built project index; if missing, direct the user to Settings › Search › 'Build index'. Never build an index from a model tool call.");
    }
    info
}

/// Resolve a project without inheriting an enclosing project's index.
/// # Errors
/// Refuses nonexistent projects and symlinked index directories.
pub fn project_root(root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("Search project must be a directory".into());
    }
    let index = root.join(".zvec-grep");
    if [
        index.clone(),
        index.join("manifest.json"),
        index.join("index"),
    ]
    .iter()
    .any(|path| std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()))
    {
        return Err("Refusing symlinked .zvec-grep index".into());
    }
    Ok(root)
}

/// Build from the trusted Settings action. No tool exposes this operation.
/// # Errors
/// Refuses absent explicit consent, unavailable CPU support, or engine failures.
#[allow(
    clippy::unused_async,
    reason = "feature-disabled build retains the same async API"
)]
pub async fn build_index(root: &Path, agent_dir: &Path, consent: bool) -> Result<(), String> {
    if !consent {
        return Err(
            "Explicit consent is required to download the model and build the index".into(),
        );
    }
    let root = project_root(root)?;
    #[cfg(feature = "zvec")]
    {
        native::build(root, agent_dir).await
    }
    #[cfg(not(feature = "zvec"))]
    {
        client::call(
            agent_dir,
            serde_json::json!({"op":"build_index", "root":root}),
        )
        .await
        .map(|_| ())
    }
}

/// Query only an existing index. This path never creates or refreshes an index.
/// # Errors
/// Refuses unavailable indices/models, path escapes, and unsupported options.
#[allow(
    clippy::unused_async,
    reason = "feature-disabled build retains the same async API"
)]
pub async fn query(
    project: &Path,
    agent_dir: &Path,
    args: &oino_ai::JsonObject,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<oino_agent::AgentToolResult, oino_agent::ToolError> {
    let root = project_root(project).map_err(oino_agent::ToolError::new)?;
    if !root.join(".zvec-grep/manifest.json").is_file() {
        return Err(oino_agent::ToolError::new(BUILD_INDEX));
    }
    #[cfg(feature = "zvec")]
    {
        native::query(root, agent_dir, args, cancel)
            .await
            .map_err(oino_agent::ToolError::new)
    }
    #[cfg(not(feature = "zvec"))]
    {
        let text = args
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| oino_agent::ToolError::new("pattern is required"))?;
        let request = serde_json::json!({"op":"query", "root":root, "text":text, "limit":args.get("head_limit")});
        let result = tokio::select! {
            () = cancel.cancelled() => return Err(oino_agent::ToolError::new("Search cancelled")),
            result = client::call(agent_dir, request) => result.map_err(oino_agent::ToolError::new)?,
        };
        serde_json::from_value(result).map_err(|e| oino_agent::ToolError::new(e.to_string()))
    }
}

#[cfg(feature = "zvec")]
mod native;

/// Number of loaded embedding models, for resource diagnostics.
#[allow(
    clippy::unused_async,
    reason = "feature-disabled build retains the same async API"
)]
pub async fn loaded_models() -> usize {
    #[cfg(feature = "zvec")]
    {
        native::loaded_models().await
    }
    #[cfg(not(feature = "zvec"))]
    {
        0
    }
}
