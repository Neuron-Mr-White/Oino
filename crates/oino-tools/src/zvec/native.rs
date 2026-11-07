//! Pinned zvec-grep library adapter. Never invokes zg or starts a daemon.
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use zg_engine::ZvecGrep;
use zg_engine::api::context::options::{
    ContextOptions, ContextRoute, ContextRouteMode, RefreshPolicy,
};
use zg_engine::api::index::options::{Device, EmbeddingModelSpec, IndexOptions};

use super::{BUILD_INDEX, IDLE_TIMEOUT, MODEL};

// Serialize engine operations so close cannot race an active model lease.
static ENGINE: OnceLock<Arc<Mutex<Option<ZvecGrep>>>> = OnceLock::new();
fn engine() -> Arc<Mutex<Option<ZvecGrep>>> {
    Arc::clone(ENGINE.get_or_init(|| Arc::new(Mutex::new(None))))
}

// A generation token makes old timers harmless without keeping the engine alive.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn schedule_retirement() {
    use std::sync::atomic::Ordering;
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let deadline = tokio::time::Instant::now() + IDLE_TIMEOUT;
    tokio::spawn(async move {
        tokio::time::sleep_until(deadline).await;
        let engine = engine();
        let mut slot = engine.lock().await;
        if GENERATION.load(Ordering::SeqCst) == generation
            && let Some(value) = slot.take()
        {
            value.close();
        }
    });
}

struct IdleGuard;
impl Drop for IdleGuard {
    fn drop(&mut self) {
        schedule_retirement();
    }
}

pub(super) async fn build(root: PathBuf, agent_dir: &Path) -> Result<(), String> {
    let engine = engine();
    let mut slot = engine.lock().await;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _idle = IdleGuard;
    let zg = slot.get_or_insert_with(ZvecGrep::new);
    let dir = root.join(".zvec-grep");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ignore = dir.join(".gitignore");
    if std::fs::symlink_metadata(&ignore).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing symlinked index .gitignore".into());
    }
    std::fs::write(ignore, "*\n").map_err(|e| e.to_string())?;
    zg.index(IndexOptions {
        root: Some(root),
        embedding: Some(EmbeddingModelSpec {
            reference: MODEL.into(),
            revision: None,
            cache_dir: Some(agent_dir.join("zvec/models")),
            endpoint: None,
            device: Device::Cpu,
        }),
        device: Some(Device::Cpu),
        model_cache: Some(agent_dir.join("zvec/models")),
        ..Default::default()
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[allow(
    clippy::too_many_lines,
    reason = "single read-only engine query with preflight and rendering"
)]
pub(super) async fn query(
    root: PathBuf,
    agent_dir: &Path,
    args: &oino_ai::JsonObject,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<oino_agent::AgentToolResult, String> {
    use zg_engine::api::info::InfoOptions;
    let engine = engine();
    let mut slot = engine.lock().await;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _idle = IdleGuard;
    let zg = slot.get_or_insert_with(ZvecGrep::new);
    async {
        let info = zg
            .info(InfoOptions {
                root: Some(root.clone()),
                include_status: false,
            })
            .await
            .map_err(|e| e.to_string())?;
        if info.root != root || !info.indexed {
            return Err(BUILD_INDEX.into());
        }
        let embedding = info
            .workspace_index
            .and_then(|i| i.embedding)
            .ok_or(BUILD_INDEX)?;
        if embedding.provider != "local" || embedding.model != "potion-code-16m-v2" {
            return Err(
                "Index must use the local CPU model. Rebuild in Settings › Search › 'Build index'."
                    .into(),
            );
        }
        let cache = agent_dir.join("zvec/models");
        let setup = zg
            .index_setup(zg_engine::api::index::setup::IndexSetupOptions {
                model_cache: Some(cache.clone()),
                preferred_model: Some(MODEL.into()),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        if !setup
            .models
            .iter()
            .any(|m| m.reference == MODEL && m.downloaded)
        {
            return Err(BUILD_INDEX.into());
        }
        let query = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or("pattern is required")?
            .to_owned();
        let mut routes = vec![ContextRoute {
            mode: ContextRouteMode::Vector,
            query: query.clone(),
        }];
        if args.get("search_mode").and_then(|v| v.as_str()) == Some("hybrid") {
            routes.push(ContextRoute {
                mode: ContextRouteMode::Fts,
                query: query.clone(),
            });
        }
        let mut options = ContextOptions {
            query: Some(query),
            routes,
            root: Some(root.clone()),
            auto_update: false,
            refresh: Some(RefreshPolicy::Off),
            device: Some(Device::Cpu),
            model_cache: Some(cache),
            authorization_model: Some(MODEL.into()),
            signal: Some(cancel),
            limit: Some(
                args.get("head_limit")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .unwrap_or(100),
            ),
            ..Default::default()
        };
        if let Some(glob) = args.get("glob_pattern").and_then(|v| v.as_str()) {
            options.filter.globs.push(glob.into());
        }
        if let Some(kind) = args.get("type").and_then(|v| v.as_str()) {
            options.filter.file_types.push(kind.into());
        }
        let scope = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let scope = root.join(scope).canonicalize().map_err(|e| e.to_string())?;
        if !scope.starts_with(&root) {
            return Err("Semantic search path must be inside the session project".into());
        }
        let found = zg.context(options).await.map_err(|e| e.to_string())?;
        let mode = args
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        let mut lines = Vec::new();
        for item in found.items {
            let path = item
                .absolute_path
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !path.starts_with(&root) || !(path == scope || path.starts_with(&scope)) {
                continue;
            }
            let relative = item.relative_path.to_string_lossy().replace('\\', "/");
            *counts.entry(relative.clone()).or_default() += 1;
            let numbered = args
                .get("line_numbers")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if numbered && let Some(line) = item.content_range.start_line() {
                lines.push(format!("{relative}:{line}: {}", item.content));
            } else {
                lines.push(format!("{relative}: {}", item.content));
            }
        }
        let text = match mode {
            "content" => lines.join("\n"),
            "count" => counts
                .into_iter()
                .map(|(p, n)| format!("{p}:{n}"))
                .collect::<Vec<_>>()
                .join("\n"),
            "files_with_matches" => counts.into_keys().collect::<Vec<_>>().join("\n"),
            _ => return Err("Invalid output_mode".into()),
        };
        let text = if text.is_empty() {
            "No matches found".into()
        } else {
            text
        };
        let truncated =
            crate::truncate::truncate_head(&text, crate::truncate::TruncationOptions::bytes_only());
        Ok(oino_agent::AgentToolResult::text(truncated.content))
    }
    .await
}

pub(super) async fn status(root: PathBuf) -> Result<serde_json::Value, String> {
    use zg_engine::api::info::InfoOptions;
    let engine = engine();
    let mut slot = engine.lock().await;
    let zg = slot.get_or_insert_with(ZvecGrep::new);
    let info = zg
        .info(InfoOptions {
            root: Some(root),
            include_status: true,
        })
        .await
        .map_err(|e| e.to_string())?;
    let state = info.index_status().as_str();
    Ok(
        serde_json::json!({"state": state, "files": info.status.as_ref().map(|s| s.files_indexed), "updated": info.workspace_index.as_ref().map(|i| i.updated_epoch_ms)}),
    )
}

/// Runtime diagnostics without loading an engine or model.
pub(super) async fn loaded_models() -> usize {
    engine()
        .lock()
        .await
        .as_ref()
        .map_or(0, |zg| zg.runtime_snapshot().loaded_models)
}
