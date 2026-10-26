//! `find` (pi `tools/find.ts`).
//!
//! pi runs `fd --glob --color=never --hidden [--no-require-git] --max-results N [--full-path]
//! -- PATTERN PATH`; Oino reproduces fd with `ignore` + `globset`: smart case, the glob matched
//! against the file name (or the absolute path when the pattern contains `/`, prefixed with
//! `**/`), `*` not crossing `/`, hidden entries included, `.gitignore` / `.ignore` / `.fdignore`
//! respected (`.gitignore` also outside git repositories), directories suffixed with `/`.
//! Results come in sorted walk order (fd's parallel order is not deterministic).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::FutureExt;
use futures::future::BoxFuture;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::JsonValue;
use serde_json::{Map, json};
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;
use crate::env::path_exists;
use crate::info::{FIND_DEFAULT_LIMIT, find_info};
use crate::js::number_to_string;
use crate::path_utils::resolve_to_cwd;
use crate::truncate::{DEFAULT_MAX_BYTES, TruncationOptions, format_size, truncate_head};
use crate::{aborted, args, js_number, node_path};

/// The `find` tool.
#[derive(Debug)]
pub struct FindTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl FindTool {
    /// A `find` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: find_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for FindTool {
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
            let stop = Arc::new(AtomicBool::new(false));
            let result = tokio::select! {
                result = run(&config, &call, Arc::clone(&stop)) => result,
                () = cancel.cancelled() => Err(aborted()),
            };
            stop.store(true, Ordering::Relaxed);
            result
        }
        .boxed()
    }
}

struct FindSpec {
    pattern: String,
    full_path: bool,
    root: String,
    require_git: bool,
    /// `0` means unlimited (fd `--max-results 0`).
    max_results: u64,
}

/// fd smart case: case-sensitive only when the pattern has an uppercase character.
fn has_uppercase(pattern: &str) -> bool {
    pattern.chars().any(char::is_uppercase)
}

fn walk(spec: &FindSpec, stop: &AtomicBool) -> Result<Vec<String>, String> {
    let glob = GlobBuilder::new(&spec.pattern)
        .literal_separator(true)
        .case_insensitive(!has_uppercase(&spec.pattern))
        .backslash_escape(!cfg!(windows))
        .build()
        .map_err(|error| format!("[fd error]: {error}"))?
        .compile_matcher();
    let mut walker = WalkBuilder::new(&spec.root);
    walker
        .hidden(false)
        .follow_links(false)
        .require_git(spec.require_git)
        .add_custom_ignore_filename(".fdignore")
        .sort_by_file_name(Ord::cmp);
    let mut results = Vec::new();
    for entry in walker.build() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path().to_string_lossy().into_owned();
        let subject = if spec.full_path {
            path.clone()
        } else {
            entry.file_name().to_string_lossy().into_owned()
        };
        if !glob.is_match(&subject) {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        results.push(if is_dir {
            format!("{path}{}", node_path::SEP)
        } else {
            path
        });
        if spec.max_results > 0 && results.len() as u64 >= spec.max_results {
            break;
        }
    }
    Ok(results)
}

/// pi `relativizeFindResultPath`.
fn relativize(result: &str, root: &str) -> String {
    let trailing = result.ends_with(node_path::SEP) || (cfg!(windows) && result.ends_with('/'));
    let relative = if node_path::is_absolute(result) {
        node_path::relative(root, result)
    } else {
        result.to_owned()
    };
    let posix = relative.replace(node_path::SEP, "/");
    if trailing && !posix.ends_with('/') {
        format!("{posix}/")
    } else {
        posix
    }
}

async fn inside_git_repo(config: &ToolConfig, root: &str) -> bool {
    let mut current = root.to_owned();
    loop {
        if path_exists(config.env.as_ref(), &node_path::join(&current, ".git")).await {
            return true;
        }
        let parent = node_path::dirname(&current);
        if parent == current {
            return false;
        }
        current = parent;
    }
}

async fn run(
    config: &ToolConfig,
    call: &ToolInvocation,
    stop: Arc<AtomicBool>,
) -> Result<AgentToolResult, ToolError> {
    let pattern = args::required_str(&call.args, "pattern")?;
    let search_dir = args::optional_str(&call.args, "path").filter(|path| !path.is_empty());
    let limit = args::optional_f64(&call.args, "limit").unwrap_or(FIND_DEFAULT_LIMIT);
    if !config.env.is_local() {
        return Err(ToolError::new(
            "find is only available on the local file system",
        ));
    }
    let root = resolve_to_cwd(search_dir.unwrap_or("."), &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    let require_git = inside_git_repo(config, &root).await;
    let is_dir = config.env.stat(&root).await.is_ok_and(|stat| stat.is_dir);
    if !is_dir {
        return Err(ToolError::new(format!(
            "[fd error]: Search path '{root}' is not a directory.\n[fd error]: No valid search paths given."
        )));
    }
    let full_path = pattern.contains('/');
    let effective =
        if full_path && !pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**"
        {
            format!("**/{pattern}")
        } else {
            pattern.to_owned()
        };
    let spec = FindSpec {
        pattern: effective,
        full_path,
        root: root.clone(),
        require_git,
        max_results: args::to_count(limit),
    };
    let results = tokio::task::spawn_blocking(move || walk(&spec, &stop))
        .await
        .map_err(|error| ToolError::new(error.to_string()))?
        .map_err(ToolError::new)?;
    if results.is_empty() {
        return Ok(AgentToolResult::text("No files found matching pattern"));
    }
    let relativized: Vec<String> = results
        .iter()
        .map(|line| line.trim_end_matches('\r'))
        .map(crate::js::trim)
        .filter(|line| !line.is_empty())
        .map(|line| relativize(line, &root))
        .collect();
    #[allow(
        clippy::cast_precision_loss,
        reason = "result counts are far below 2^53"
    )]
    let limit_reached = relativized.len() as f64 >= limit;
    let truncation = truncate_head(&relativized.join("\n"), TruncationOptions::bytes_only());
    let mut output = truncation.content.clone();
    let mut details = Map::new();
    let mut notices = Vec::new();
    if limit_reached {
        notices.push(format!(
            "{} results limit reached. Use limit={} for more, or refine pattern",
            number_to_string(limit),
            number_to_string(limit * 2.0)
        ));
        details.insert("resultLimitReached".to_owned(), js_number(limit));
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
